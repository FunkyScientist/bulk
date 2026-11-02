//! `bulk find`: search interactively, like fzf, with sizes, a preview and actions.
//! An empty search browses folders instead. Draws on stderr, so what Enter prints
//! on stdout can be piped.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime};

use bulk_core::drive::{Fresh, Loaded};
use bulk_core::helper::{CatchUp, Client};
use bulk_core::kinds;
use bulk_core::search::{self, Sort};
use bulk_core::tree::{NodeId, Tree, flags};
use bulk_core::usn::{self, FrnIndex};
use clap::Args;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::delete::{self, Target};
use crate::load::{self, Freshness, Loader, Status};
use crate::out;
use crate::read;
use crate::theme::Theme;

#[derive(Args, Default)]
pub struct FindArgs {
    /// Search to start with
    query: Vec<String>,
    /// Folder or drive to look in (default: the current folder)
    #[arg(long = "in", value_name = "PATH")]
    within: Option<PathBuf>,
    /// Separate the printed paths with NUL instead of new lines
    #[arg(long, conflicts_with = "json")]
    print0: bool,
    #[command(flatten)]
    fresh: Freshness,
}

/// Most results one search lists.
const LIMIT: usize = 1000;

// --- Data shared with the loading and watching threads --------------------------

struct Drive {
    l: Loaded,
    index: Option<FrnIndex>,
    /// The change journal is being watched, so the tree is live.
    watched: bool,
}

struct Shared {
    drives: Mutex<Vec<Drive>>,
    /// Bumped whenever a tree changes or a drive finishes loading.
    generation: AtomicU64,
    status: Status,
    stop: AtomicBool,
}

impl Shared {
    fn bump(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

/// Loads drives as they're asked for, then keeps each one live.
fn loader(shared: Arc<Shared>, fresh: Freshness, wanted: mpsc::Receiver<char>) {
    let loader = Loader::quiet(&fresh, shared.status.clone());
    let mut asked: Vec<char> = Vec::new();
    while let Ok(letter) = wanted.recv() {
        if asked.contains(&letter) {
            continue;
        }
        asked.push(letter);
        match loader.drive(letter) {
            Ok(l) => {
                let live = l.fresh != Fresh::Snapshot && l.meta.journal.is_some();
                shared.drives.lock().unwrap().push(Drive {
                    l,
                    index: None,
                    watched: false,
                });
                shared.status.lock().unwrap().clear();
                shared.bump();
                if live {
                    let s = shared.clone();
                    std::thread::spawn(move || watch(s, letter));
                }
            }
            Err(e) => *shared.status.lock().unwrap() = format!("{letter}: {e:#}"),
        }
    }
}

/// Apply the drive's journal as it changes, until the finder closes.
fn watch(shared: Arc<Shared>, letter: char) {
    let Ok(client) = Client::new() else { return };
    let set_watched = |on: bool| {
        if let Some(d) = shared
            .drives
            .lock()
            .unwrap()
            .iter_mut()
            .find(|d| d.l.vol.letter == letter)
        {
            d.watched = on;
        }
        shared.bump();
    };
    set_watched(true);
    while !shared.stop.load(Ordering::Relaxed) {
        let journal = {
            let drives = shared.drives.lock().unwrap();
            drives
                .iter()
                .find(|d| d.l.vol.letter == letter)
                .and_then(|d| d.l.meta.journal)
        };
        let Some(j) = journal else { break };
        let r = client.watch(letter, j.id, j.next, |changes, next| {
            let mut drives = shared.drives.lock().unwrap();
            if let Some(d) = drives.iter_mut().find(|d| d.l.vol.letter == letter) {
                if !changes.is_empty() {
                    let tree = &mut d.l.tree;
                    let index = d.index.get_or_insert_with(|| FrnIndex::build(tree));
                    if usn::apply(tree, index, &changes) > 0 {
                        d.l.meta.taken = bulk_core::os::now();
                        shared.bump();
                    }
                }
                if let Some(j) = d.l.meta.journal.as_mut() {
                    j.next = next;
                }
            }
            !shared.stop.load(Ordering::Relaxed)
        });
        match r {
            Ok(CatchUp::Gap) => break,
            Ok(_) => {}
            Err(_) => {
                std::thread::sleep(Duration::from_secs(3));
                if !client.ensure_running(Duration::from_secs(5)) {
                    break;
                }
            }
        }
    }
    set_watched(false);
}

// --- The finder's own state -----------------------------------------------------

#[derive(Clone, PartialEq)]
enum Scope {
    Folder(char, String),
    Drive(char),
    All,
}

#[derive(Clone, Copy, PartialEq)]
struct Row {
    letter: char,
    id: NodeId,
}

const SORTS: [(Sort, &str); 4] = [
    (Sort::Relevance, "best match"),
    (Sort::Size, "size"),
    (Sort::Newest, "newest"),
    (Sort::Name, "name"),
];

struct Ui {
    input: String,
    /// Cursor in `input`, in chars.
    cur: usize,
    scope: Scope,
    /// Where the finder started; Ctrl+R comes back to it.
    home: Scope,
    sort: usize,
    rows: Vec<Row>,
    total: usize,
    took: Duration,
    sel: usize,
    offset: usize,
    picked: Vec<Row>,
    confirm: Option<Confirm>,
    help: bool,
    msg: Option<(String, bool, Instant)>,
    seen: u64,
    history: Vec<String>,
    hist_pos: Option<usize>,
    /// List area from the last draw, for mouse clicks.
    list_area: Rect,
    /// Type breakdowns for the preview, by row.
    types: HashMap<(char, NodeId), (u64, kinds::Breakdown)>,
}

struct Confirm {
    targets: Vec<(Row, Target)>,
    refused: Vec<(String, String)>,
}

enum Exit {
    Quit,
    Print(Vec<Row>),
}

pub fn find(a: FindArgs) -> anyhow::Result<()> {
    let start = match &a.within {
        Some(p) => load::absolute(p)?,
        None => load::absolute(Path::new("."))?,
    };
    let letter = load::letter_of(&start)?;
    let home = if start.len() <= 3 {
        Scope::Drive(letter)
    } else {
        Scope::Folder(letter, start)
    };

    let shared = Arc::new(Shared {
        drives: Mutex::new(Vec::new()),
        generation: AtomicU64::new(0),
        status: Arc::new(Mutex::new(format!("Loading {letter}:…"))),
        stop: AtomicBool::new(false),
    });
    let (want, wanted) = mpsc::channel();
    let _ = want.send(letter);
    {
        let (s, f) = (shared.clone(), a.fresh.clone());
        std::thread::spawn(move || loader(s, f, wanted));
    }

    let input = a.query.join(" ");
    let mut ui = Ui {
        cur: input.chars().count(),
        input,
        scope: home.clone(),
        home,
        sort: 0,
        rows: Vec::new(),
        total: 0,
        took: Duration::ZERO,
        sel: 0,
        offset: 0,
        picked: Vec::new(),
        confirm: None,
        help: false,
        msg: None,
        seen: u64::MAX,
        history: load_history(),
        hist_pos: None,
        list_area: Rect::default(),
        types: HashMap::new(),
    };
    let theme = Theme::load();

    let mut term = setup()?;
    let r = run(&mut term, &mut ui, &shared, &want, &theme);
    restore_terminal(&mut term);
    shared.stop.store(true, Ordering::Relaxed);

    match r? {
        Exit::Quit => Ok(()),
        Exit::Print(rows) => {
            save_history(&ui.history, &ui.input);
            let drives = shared.drives.lock().unwrap();
            let paths: Vec<(String, Option<read::Item>)> = rows
                .iter()
                .filter_map(|r| {
                    let t = &drive(&drives, r.letter)?.l.tree;
                    Some((t.path(r.id), a.fresh.json.then(|| read::item(t, r.id))))
                })
                .collect();
            if a.fresh.json {
                return out::json(&paths.into_iter().filter_map(|(_, i)| i).collect::<Vec<_>>());
            }
            let mut stdout = std::io::stdout().lock();
            for (p, _) in paths {
                if a.print0 {
                    write!(stdout, "{p}\0")?;
                } else {
                    writeln!(stdout, "{p}")?;
                }
            }
            Ok(())
        }
    }
}

type Term = Terminal<CrosstermBackend<std::io::Stderr>>;

fn setup() -> anyhow::Result<Term> {
    enable_raw_mode()?;
    let mut err = std::io::stderr();
    execute!(err, EnterAlternateScreen, EnableMouseCapture)?;
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stderr(), LeaveAlternateScreen, DisableMouseCapture);
        prev(info);
    }));
    Ok(Terminal::new(CrosstermBackend::new(err))?)
}

fn restore_terminal(term: &mut Term) {
    let _ = disable_raw_mode();
    let _ = execute!(
        term.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = term.show_cursor();
}

fn drive(drives: &[Drive], letter: char) -> Option<&Drive> {
    drives.iter().find(|d| d.l.vol.letter == letter)
}

fn run(
    term: &mut Term,
    ui: &mut Ui,
    shared: &Shared,
    want: &mpsc::Sender<char>,
    theme: &Theme,
) -> anyhow::Result<Exit> {
    let mut last_query = String::from("\0");
    let mut last_scope = ui.scope.clone();
    let mut last_sort = usize::MAX;
    loop {
        // Search again when the query, scope or sort changed, or the trees did.
        let generation = shared.generation.load(Ordering::Relaxed);
        if generation != ui.seen
            || ui.input != last_query
            || ui.scope != last_scope
            || ui.sort != last_sort
        {
            let drives = shared.drives.lock().unwrap();
            refresh(ui, &drives);
            ui.seen = generation;
            last_query = ui.input.clone();
            last_scope = ui.scope.clone();
            last_sort = ui.sort;
        }
        {
            let drives = shared.drives.lock().unwrap();
            let status = shared.status.lock().unwrap().clone();
            term.draw(|f| draw(f, ui, &drives, theme, &status))?;
        }

        if !event::poll(Duration::from_millis(120))? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                if let Some(exit) = key(ui, k, shared, want)? {
                    return Ok(exit);
                }
            }
            Event::Mouse(m) => match m.kind {
                MouseEventKind::ScrollDown => move_sel(ui, 3),
                MouseEventKind::ScrollUp => move_sel(ui, -3),
                MouseEventKind::Down(MouseButton::Left) => {
                    let a = ui.list_area;
                    if m.column >= a.x
                        && m.column < a.x + a.width
                        && m.row >= a.y
                        && m.row < a.y + a.height
                    {
                        let i = ui.offset + (m.row - a.y) as usize;
                        if i < ui.rows.len() {
                            ui.sel = i;
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// The current folder for browsing and for searches limited to a folder.
fn scope_node(drives: &[Drive], scope: &Scope) -> Option<(char, NodeId)> {
    match scope {
        Scope::Folder(l, p) => Some((*l, drive(drives, *l)?.l.tree.find_path(p)?)),
        Scope::Drive(l) => Some((*l, drive(drives, *l)?.l.tree.root)),
        Scope::All => None,
    }
}

fn refresh(ui: &mut Ui, drives: &[Drive]) {
    let keep = ui.rows.get(ui.sel).copied();
    let t0 = Instant::now();
    let sort = SORTS[ui.sort].0;
    let text = ui.input.trim();

    let (rows, total) = if text.is_empty() {
        browse_rows(drives, &ui.scope, sort)
    } else {
        let mut q = search::parse(text, bulk_core::os::now());
        if !text.contains("sort:") {
            q.sort = sort;
        }
        let scope = scope_node(drives, &ui.scope);
        let mut hits: Vec<(u8, Row)> = Vec::new();
        let mut total = 0;
        for d in drives {
            let letter = d.l.vol.letter;
            let within = match scope {
                Some((l, _)) if l != letter => continue,
                Some((_, id)) => Some(id),
                None if !q.drives.is_empty() && !q.drives.contains(&letter) => continue,
                None => None,
            };
            let (h, n) = search::search(&d.l.tree, &q, within, LIMIT);
            total += n;
            hits.extend(h.into_iter().map(|h| (h.score, Row { letter, id: h.id })));
        }
        let rank = |(score, r): &(u8, Row)| {
            let t = &drive(drives, r.letter).unwrap().l.tree;
            let n = t.get(r.id);
            search::Rank {
                score: *score,
                alloc: n.alloc,
                mtime: n.mtime,
                files: n.files,
                name: t.name(r.id),
            }
        };
        hits.sort_by(|a, b| search::compare(q.sort, &rank(a), &rank(b)));
        hits.truncate(LIMIT);
        (hits.into_iter().map(|(_, r)| r).collect(), total)
    };
    ui.rows = rows;
    ui.total = total;
    ui.took = t0.elapsed();
    // Stay on the same item when it's still there.
    ui.sel = keep
        .and_then(|k| ui.rows.iter().position(|r| *r == k))
        .unwrap_or(0);
    // Forget picks that no longer exist.
    ui.picked
        .retain(|r| drive(drives, r.letter).is_some_and(|d| !d.l.tree.get(r.id).has(flags::GONE)));
}

fn browse_rows(drives: &[Drive], scope: &Scope, sort: Sort) -> (Vec<Row>, usize) {
    let Some((letter, id)) = scope_node(drives, scope) else {
        // All drives: their roots.
        let rows: Vec<Row> = drives
            .iter()
            .map(|d| Row {
                letter: d.l.vol.letter,
                id: d.l.tree.root,
            })
            .collect();
        let n = rows.len();
        return (rows, n);
    };
    let tree = &drive(drives, letter).unwrap().l.tree;
    let mut kids: Vec<NodeId> = tree
        .children(id)
        .filter(|&c| !tree.get(c).has(flags::GONE))
        .collect();
    match sort {
        Sort::Name => kids.sort_by_key(|&c| tree.name(c).to_lowercase()),
        Sort::Newest => kids.sort_by_key(|&c| std::cmp::Reverse(tree.get(c).mtime)),
        _ => kids.sort_by_key(|&c| std::cmp::Reverse(tree.get(c).alloc)),
    }
    let n = kids.len();
    (kids.into_iter().map(|id| Row { letter, id }).collect(), n)
}

fn move_sel(ui: &mut Ui, by: isize) {
    if ui.rows.is_empty() {
        return;
    }
    let max = ui.rows.len() as isize - 1;
    ui.sel = (ui.sel as isize + by).clamp(0, max) as usize;
}

fn say(ui: &mut Ui, msg: impl Into<String>, error: bool) {
    ui.msg = Some((msg.into(), error, Instant::now()));
}

fn set_input(ui: &mut Ui, s: String) {
    ui.cur = s.chars().count();
    ui.input = s;
}

/// Byte offset of char `i` in `s`.
fn byte_at(s: &str, i: usize) -> usize {
    s.char_indices().nth(i).map_or(s.len(), |(b, _)| b)
}

/// What the actions work on: the picked rows, or else the highlighted one.
fn acting_on(ui: &Ui) -> Vec<Row> {
    if ui.picked.is_empty() {
        ui.rows.get(ui.sel).copied().into_iter().collect()
    } else {
        ui.picked.clone()
    }
}

fn paths(ui: &Ui, drives: &[Drive]) -> Vec<String> {
    acting_on(ui)
        .iter()
        .filter_map(|r| Some(drive(drives, r.letter)?.l.tree.path(r.id)))
        .collect()
}

fn key(
    ui: &mut Ui,
    k: KeyEvent,
    shared: &Shared,
    want: &mpsc::Sender<char>,
) -> anyhow::Result<Option<Exit>> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);

    if ui.help {
        ui.help = false;
        return Ok(None);
    }
    if let Some(c) = &ui.confirm {
        match k.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                let rows = c.targets.clone();
                ui.confirm = None;
                recycle(ui, shared, rows);
            }
            _ => ui.confirm = None,
        }
        return Ok(None);
    }

    let len = ui.input.chars().count();
    match k.code {
        KeyCode::Char('c' | 'q') if ctrl => return Ok(Some(Exit::Quit)),
        KeyCode::Esc => {
            if ui.input.is_empty() {
                return Ok(Some(Exit::Quit));
            }
            set_input(ui, String::new());
        }
        KeyCode::Enter => {
            let rows = acting_on(ui);
            if !rows.is_empty() {
                return Ok(Some(Exit::Print(rows)));
            }
        }
        KeyCode::F(1) => ui.help = true,
        KeyCode::Char('?') if ui.input.is_empty() => ui.help = true,

        // Moving through the list.
        KeyCode::Up => move_sel(ui, -1),
        KeyCode::Down => move_sel(ui, 1),
        KeyCode::PageUp => move_sel(ui, -(ui.list_area.height.max(2) as isize - 1)),
        KeyCode::PageDown => move_sel(ui, ui.list_area.height.max(2) as isize - 1),
        KeyCode::Home if ctrl => ui.sel = 0,
        KeyCode::End if ctrl => ui.sel = ui.rows.len().saturating_sub(1),

        // Picking.
        KeyCode::Tab | KeyCode::BackTab => {
            if let Some(&r) = ui.rows.get(ui.sel) {
                match ui.picked.iter().position(|p| *p == r) {
                    Some(i) => {
                        ui.picked.remove(i);
                    }
                    None => ui.picked.push(r),
                }
                move_sel(ui, if k.code == KeyCode::BackTab { -1 } else { 1 });
            }
        }
        KeyCode::Char('a') if ctrl => {
            if ui.rows.iter().all(|r| ui.picked.contains(r)) {
                ui.picked.retain(|p| !ui.rows.contains(p));
            } else {
                for &r in &ui.rows {
                    if !ui.picked.contains(&r) {
                        ui.picked.push(r);
                    }
                }
            }
        }

        // Actions.
        KeyCode::Char('o') if ctrl => {
            let drives = shared.drives.lock().unwrap();
            if let Some(p) = paths(ui, &drives).first() {
                let r = bulk_core::os::open(p);
                drop(drives);
                match r {
                    Ok(()) => say(ui, "Opened", false),
                    Err(e) => say(ui, format!("{e:#}"), true),
                }
            }
        }
        KeyCode::Char('e') if ctrl => {
            let p = paths(ui, &shared.drives.lock().unwrap()).into_iter().next();
            if let Some(p) = p {
                let _ = bulk_core::os::reveal(&p);
            }
        }
        KeyCode::Char('b') if ctrl => {
            let p = paths(ui, &shared.drives.lock().unwrap()).into_iter().next();
            if let Some(p) = p {
                match bulk_core::os::open_in_app(&p) {
                    Ok(()) => say(ui, "Opening in Bulk", false),
                    Err(e) => say(ui, format!("{e:#}"), true),
                }
            }
        }
        KeyCode::Char('c') if alt => {
            let p = paths(ui, &shared.drives.lock().unwrap());
            if !p.is_empty() {
                match copy(&p.join("\r\n")) {
                    Ok(()) => say(
                        ui,
                        if p.len() == 1 {
                            "Copied the path".to_string()
                        } else {
                            format!("Copied {} paths", p.len())
                        },
                        false,
                    ),
                    Err(e) => say(ui, format!("Couldn't copy: {e:#}"), true),
                }
            }
        }
        KeyCode::Char('d') if ctrl => {
            let drives = shared.drives.lock().unwrap();
            ui.confirm = confirm(ui, &drives);
        }
        KeyCode::Char('r') if ctrl => {
            ui.scope = match &ui.scope {
                Scope::Folder(l, _) => Scope::Drive(*l),
                Scope::Drive(_) => Scope::All,
                Scope::All => ui.home.clone(),
            };
            if ui.scope == Scope::All {
                for v in bulk_core::volume::list() {
                    if v.auto_scan() && bulk_core::snapshot::current_path(v.serial).exists() {
                        let _ = want.send(v.letter);
                    }
                }
            }
        }
        KeyCode::Char('s') if ctrl => ui.sort = (ui.sort + 1) % SORTS.len(),
        KeyCode::Char('p') if ctrl => history(ui, 1),
        KeyCode::Char('n') if ctrl => history(ui, -1),

        // Folders: into one with → at the end of the search, up with ← at its start.
        KeyCode::Right if ui.cur == len => {
            let drives = shared.drives.lock().unwrap();
            if let Some(&r) = ui.rows.get(ui.sel)
                && let Some(d) = drive(&drives, r.letter)
                && d.l.tree.get(r.id).is_dir()
            {
                let path = d.l.tree.path(r.id);
                drop(drives);
                remember(ui);
                set_input(ui, String::new());
                ui.scope = if r.id == 0 {
                    Scope::Drive(r.letter)
                } else {
                    Scope::Folder(r.letter, path)
                };
                ui.sel = 0;
            }
        }
        KeyCode::Left if ui.cur == 0 => {
            let up = match &ui.scope {
                Scope::Folder(l, p) => Some(match p.rsplit_once('\\') {
                    Some((parent, _)) if parent.len() > 2 => Scope::Folder(*l, parent.into()),
                    _ => Scope::Drive(*l),
                }),
                Scope::Drive(_) => Some(Scope::All),
                Scope::All => None,
            };
            if let Some(up) = up {
                let came_from = match &ui.scope {
                    Scope::Folder(l, p) => Some((*l, p.clone())),
                    _ => None,
                };
                ui.scope = up;
                if ui.scope == Scope::All {
                    for v in bulk_core::volume::list() {
                        if v.auto_scan() && bulk_core::snapshot::current_path(v.serial).exists() {
                            let _ = want.send(v.letter);
                        }
                    }
                }
                // Land on the folder we came out of.
                let drives = shared.drives.lock().unwrap();
                refresh(ui, &drives);
                if let Some((l, p)) = came_from
                    && let Some(id) = drive(&drives, l).and_then(|d| d.l.tree.find_path(&p))
                {
                    ui.sel = ui
                        .rows
                        .iter()
                        .position(|r| r.letter == l && r.id == id)
                        .unwrap_or(0);
                }
            }
        }

        // Editing the search.
        KeyCode::Left => ui.cur = ui.cur.saturating_sub(1),
        KeyCode::Right => ui.cur = (ui.cur + 1).min(len),
        KeyCode::Home => ui.cur = 0,
        KeyCode::End => ui.cur = len,
        KeyCode::Backspace if ctrl || alt => {
            // A word back.
            let before: String = ui.input.chars().take(ui.cur).collect();
            let trimmed = before.trim_end();
            let cut = trimmed.rfind(' ').map_or(0, |i| i + 1);
            let after: String = ui.input.chars().skip(ui.cur).collect();
            ui.cur = trimmed[..cut].chars().count();
            ui.input = format!("{}{after}", &trimmed[..cut]);
        }
        KeyCode::Backspace if ui.cur > 0 => {
            let b = byte_at(&ui.input, ui.cur - 1);
            ui.input.remove(b);
            ui.cur -= 1;
        }
        KeyCode::Delete if ui.cur < len => {
            let b = byte_at(&ui.input, ui.cur);
            ui.input.remove(b);
        }
        KeyCode::Char('u') if ctrl => set_input(ui, String::new()),
        KeyCode::Char(c) if !ctrl && !alt => {
            let b = byte_at(&ui.input, ui.cur);
            ui.input.insert(b, c);
            ui.cur += 1;
            ui.hist_pos = None;
        }
        _ => {}
    }
    Ok(None)
}

/// Ask before deleting: what would go, and what can't.
fn confirm(ui: &mut Ui, drives: &[Drive]) -> Option<Confirm> {
    let mut targets = Vec::new();
    let mut refused = Vec::new();
    for r in acting_on(ui) {
        let Some(d) = drive(drives, r.letter) else {
            continue;
        };
        let path = d.l.tree.path(r.id);
        let why = bulk_core::trash::protected(Path::new(&path))
            .or_else(|| delete::node_refusal(&d.l, r.id));
        match why {
            Some(why) => refused.push((path, why.to_string())),
            None => {
                let n = d.l.tree.get(r.id);
                targets.push((
                    r,
                    Target {
                        path,
                        dir: n.is_dir(),
                        alloc: Some(n.alloc),
                    },
                ));
            }
        }
    }
    // Leave out what's inside another target.
    let keep: Vec<String> = delete::topmost(targets.iter().map(|(_, t)| t.clone()).collect())
        .into_iter()
        .map(|t| t.path)
        .collect();
    targets.retain(|(_, t)| keep.contains(&t.path));
    if targets.is_empty() && refused.is_empty() {
        return None;
    }
    Some(Confirm { targets, refused })
}

fn recycle(ui: &mut Ui, shared: &Shared, rows: Vec<(Row, Target)>) {
    let started = SystemTime::now();
    let mut done = Vec::new();
    let mut failed = 0;
    for (r, t) in rows {
        match bulk_core::trash::recycle(Path::new(&t.path)) {
            Ok(()) => {
                let mut drives = shared.drives.lock().unwrap();
                if let Some(d) = drives.iter_mut().find(|d| d.l.vol.letter == r.letter) {
                    d.l.tree.mark_gone(r.id);
                }
                done.push(t);
            }
            Err(_) => failed += 1,
        }
    }
    shared.bump();
    ui.picked.clear();
    let total: u64 = done.iter().filter_map(|t| t.alloc).sum();
    let id = delete::log_batch(started, false, &done).ok();
    let mut msg = format!(
        "Moved {} to the Recycle Bin ({})",
        if done.len() == 1 {
            "1 item".to_string()
        } else {
            format!("{} items", done.len())
        },
        out::size(total)
    );
    if let Some(id) = id {
        msg.push_str(&format!(" · undo: bulk restore {id}"));
    }
    if failed > 0 {
        msg.push_str(&format!(" · {failed} couldn't be deleted"));
    }
    say(ui, msg, failed > 0);
}

fn remember(ui: &mut Ui) {
    let q = ui.input.trim().to_string();
    if !q.is_empty() && ui.history.last() != Some(&q) {
        ui.history.retain(|h| *h != q);
        ui.history.push(q);
    }
}

/// Step through earlier searches: 1 is older, -1 newer.
fn history(ui: &mut Ui, dir: isize) {
    if ui.history.is_empty() {
        return;
    }
    let n = ui.history.len();
    let pos = match (ui.hist_pos, dir) {
        (None, 1) => n - 1,
        (None, _) => return,
        (Some(p), 1) => p.saturating_sub(1),
        (Some(p), _) if p + 1 >= n => {
            ui.hist_pos = None;
            set_input(ui, String::new());
            return;
        }
        (Some(p), _) => p + 1,
    };
    ui.hist_pos = Some(pos);
    set_input(ui, ui.history[pos].clone());
}

fn history_path() -> PathBuf {
    bulk_core::snapshot::root().with_file_name("find-history.txt")
}

fn load_history() -> Vec<String> {
    std::fs::read_to_string(history_path())
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn save_history(h: &[String], last: &str) {
    let mut h: Vec<String> = h.to_vec();
    let last = last.trim();
    if !last.is_empty() {
        h.retain(|x| x != last);
        h.push(last.to_string());
    }
    let start = h.len().saturating_sub(200);
    let _ = std::fs::write(history_path(), h[start..].join("\n"));
}

/// Put `text` on the clipboard.
fn copy(text: &str) -> anyhow::Result<()> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    const CF_UNICODETEXT: u32 = 13;
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        anyhow::ensure!(
            OpenClipboard(std::ptr::null_mut()) != 0,
            "clipboard is busy"
        );
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        let ok = !mem.is_null() && {
            let p = GlobalLock(mem) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                GlobalUnlock(mem);
                !SetClipboardData(CF_UNICODETEXT, mem).is_null()
            } else {
                false
            }
        };
        CloseClipboard();
        anyhow::ensure!(ok, "couldn't write to the clipboard");
    }
    Ok(())
}

// --- Drawing --------------------------------------------------------------------

fn kind_key(tree: &Tree, id: NodeId) -> &'static str {
    if tree.get(id).is_dir() {
        "folder"
    } else {
        kinds::kind_key(kinds::kind_of(tree.name_lower(id)))
    }
}

fn scope_label(scope: &Scope) -> String {
    match scope {
        Scope::Folder(_, p) => p.clone(),
        Scope::Drive(l) => format!("{l}:\\"),
        Scope::All => "all drives".into(),
    }
}

/// Fit `s` in `w` columns, cutting from the left (paths keep their end).
fn fit_left(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w {
        return s.to_string();
    }
    if w < 2 {
        return String::new();
    }
    let tail: String = s.chars().skip(n - (w - 1)).collect();
    format!("…{tail}")
}

fn fit_right(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w {
        return s.to_string();
    }
    let head: String = s.chars().take(w.saturating_sub(1)).collect();
    format!("{head}…")
}

fn draw(f: &mut ratatui::Frame, ui: &mut Ui, drives: &[Drive], t: &Theme, status: &str) {
    let area = f.area();
    let [top, main, foot, keys] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let muted = Style::new().fg(t.muted);
    let subtle = Style::new().fg(t.subtle);
    let accent = Style::new().fg(t.accent);

    // Search line, with where it looks on the right.
    let scope = scope_label(&ui.scope);
    let live = match &ui.scope {
        Scope::All => drives.iter().all(|d| d.watched) && !drives.is_empty(),
        Scope::Folder(l, _) | Scope::Drive(l) => drive(drives, *l).is_some_and(|d| d.watched),
    };
    let fresh = match &ui.scope {
        Scope::Folder(l, _) | Scope::Drive(l) => drive(drives, *l).map(|d| {
            if d.watched {
                "live".to_string()
            } else {
                load::freshness(&d.l)
            }
        }),
        Scope::All => live.then(|| "live".to_string()),
    };
    let right = format!(
        " in {}{} ",
        fit_left(&scope, (top.width as usize / 2).max(10)),
        fresh.map(|f| format!(" · {f}")).unwrap_or_default()
    );
    let rw = right.chars().count() as u16;
    let [input_area, scope_area] =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(rw)])
            .areas(Rect { height: 1, ..top });
    let prompt = Span::styled("❯ ", accent.add_modifier(Modifier::BOLD));
    let shown = if ui.input.is_empty() {
        Span::styled(
            "Search, or browse with → and ←   (F1: help)",
            subtle.add_modifier(Modifier::ITALIC),
        )
    } else {
        Span::raw(ui.input.as_str())
    };
    f.render_widget(Paragraph::new(Line::from(vec![prompt, shown])), input_area);
    f.render_widget(
        Paragraph::new(Line::styled(
            right,
            if live { Style::new().fg(t.ok) } else { muted },
        )),
        scope_area,
    );
    f.set_cursor_position((input_area.x + 2 + ui.cur as u16, input_area.y));
    f.render_widget(
        Block::new().borders(Borders::TOP).border_style(subtle),
        Rect {
            y: top.y + 1,
            height: 1,
            ..top
        },
    );

    // Results, and the preview beside them when there's room.
    let (list, preview) = if main.width >= 110 {
        let [l, p] = Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
            .areas(main);
        (l, Some(p))
    } else {
        (main, None)
    };
    ui.list_area = list;
    let h = list.height as usize;
    if ui.sel < ui.offset {
        ui.offset = ui.sel;
    } else if ui.sel >= ui.offset + h {
        ui.offset = ui.sel + 1 - h;
    }
    ui.offset = ui.offset.min(ui.rows.len().saturating_sub(h));

    let browsing = ui.input.trim().is_empty();
    let base = scope_node(drives, &ui.scope);
    let biggest = ui
        .rows
        .iter()
        .filter_map(|r| Some(drive(drives, r.letter)?.l.tree.get(r.id).alloc))
        .max()
        .unwrap_or(0)
        .max(1);
    let mut lines = Vec::new();
    for (i, r) in ui.rows.iter().enumerate().skip(ui.offset).take(h) {
        let Some(d) = drive(drives, r.letter) else {
            continue;
        };
        let tree = &d.l.tree;
        let n = tree.get(r.id);
        let picked = ui.picked.contains(r);
        let colour = t.file(kind_key(tree, r.id));
        let name = if browsing && base.is_some() {
            let mut s = tree.name(r.id).to_string();
            if n.is_dir() {
                s.push('\\');
            }
            s
        } else if let Some((_, b)) = base.filter(|(l, _)| *l == r.letter) {
            read::rel(tree, b, r.id)
        } else {
            let mut s = tree.path(r.id);
            if n.is_dir() && r.id != tree.root {
                s.push('\\');
            }
            s
        };
        let width = list.width as usize;
        let fixed = 2 + 10 + 7 + 11;
        let frac = n.alloc as f64 / biggest as f64;
        let mut row_style = Style::new();
        if i == ui.sel {
            row_style = row_style
                .bg(Color::Rgb(45, 45, 58))
                .add_modifier(Modifier::BOLD);
        }
        lines.push(
            Line::from(vec![
                Span::styled(if picked { "● " } else { "  " }, accent),
                Span::raw(format!("{:>9} ", out::size(n.alloc))),
                Span::styled(out::bar(frac, 6), Style::new().fg(colour)),
                Span::raw(" "),
                Span::styled(format!("{:<10} ", out::date(n.mtime)), subtle),
                Span::styled(
                    fit_left(&name, width.saturating_sub(fixed)),
                    if n.is_dir() {
                        Style::new().fg(t.file("folder"))
                    } else {
                        Style::new()
                    },
                ),
            ])
            .style(row_style),
        );
    }
    if ui.rows.is_empty() {
        let msg = if drives.is_empty() || !status.is_empty() {
            status.to_string()
        } else if browsing {
            "This folder is empty.".into()
        } else {
            "Nothing matches.".into()
        };
        lines.push(Line::styled(format!("  {msg}"), muted));
    }
    f.render_widget(Paragraph::new(lines), list);

    if let Some(p) = preview {
        draw_preview(f, ui, drives, t, p);
    }

    // Counts, picks and messages.
    let mut left = if browsing {
        match base {
            Some((l, id)) => {
                let n = drive(drives, l).unwrap().l.tree.get(id);
                format!(
                    " {} items · {} on disk · {} files",
                    out::count(ui.total as u64),
                    out::size(n.alloc),
                    out::count(n.files as u64)
                )
            }
            None => format!(" {} drives", ui.rows.len()),
        }
    } else {
        format!(
            " {} match{} · {} ms",
            out::count(ui.total as u64),
            if ui.total == 1 { "" } else { "es" },
            ui.took.as_millis()
        )
    };
    left.push_str(&format!(" · sort: {}", SORTS[ui.sort].1));
    let mut spans = vec![Span::styled(left, muted)];
    if !ui.picked.is_empty() {
        let size: u64 = ui
            .picked
            .iter()
            .filter_map(|r| Some(drive(drives, r.letter)?.l.tree.get(r.id).alloc))
            .sum();
        spans.push(Span::styled(
            format!("   {} picked · {}", ui.picked.len(), out::size(size)),
            accent.add_modifier(Modifier::BOLD),
        ));
    }
    if let Some((m, err, at)) = &ui.msg {
        if at.elapsed() < Duration::from_secs(6) {
            spans.push(Span::styled(
                format!("   {m}"),
                Style::new().fg(if *err { t.danger } else { t.ok }),
            ));
        }
    } else if !status.is_empty() && !drives.is_empty() {
        spans.push(Span::styled(format!("   {status}"), muted));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), foot);

    let hint = |k: &'static str, what: &'static str| {
        vec![
            Span::styled(k, Style::new().fg(t.muted).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {what}  "), subtle),
        ]
    };
    let hints: Vec<Span> = [
        ("Tab", "pick"),
        ("Enter", "print"),
        ("^O", "open"),
        ("^E", "explorer"),
        ("^B", "app"),
        ("^D", "delete"),
        ("^R", "scope"),
        ("^S", "sort"),
        ("F1", "help"),
    ]
    .into_iter()
    .flat_map(|(k, w)| hint(k, w))
    .collect();
    f.render_widget(
        Paragraph::new(Line::from([vec![Span::raw(" ")], hints].concat())),
        keys,
    );

    if let Some(c) = &ui.confirm {
        draw_confirm(f, c, t);
    }
    if ui.help {
        draw_help(f, t);
    }
}

fn draw_preview(f: &mut ratatui::Frame, ui: &mut Ui, drives: &[Drive], t: &Theme, area: Rect) {
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(Style::new().fg(t.subtle));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(&r) = ui.rows.get(ui.sel) else {
        return;
    };
    let Some(d) = drive(drives, r.letter) else {
        return;
    };
    let tree = &d.l.tree;
    let n = *tree.get(r.id);
    let muted = Style::new().fg(t.muted);
    let subtle = Style::new().fg(t.subtle);
    let w = inner.width.saturating_sub(2) as usize;
    let kind = kind_key(tree, r.id);

    let mut lines = vec![
        Line::styled(
            format!(" {}", fit_right(tree.name(r.id), w)),
            Style::new().fg(t.file(kind)).add_modifier(Modifier::BOLD),
        ),
        Line::styled(format!(" {}", fit_left(&tree.path(r.id), w)), subtle),
        Line::raw(""),
    ];
    let fact = |k: &str, v: String| {
        Line::from(vec![Span::styled(format!(" {k:<13}"), muted), Span::raw(v)])
    };
    lines.push(fact("On disk", out::size(n.alloc)));
    lines.push(fact("Size", out::size(n.size)));
    if n.mtime > 0 {
        lines.push(fact(
            "Modified",
            format!("{} ({})", out::date(n.mtime), out::ago(n.mtime as u64)),
        ));
    }
    if n.is_dir() {
        lines.push(fact("Files", out::count(n.files as u64)));
    } else {
        lines.push(fact(
            "Kind",
            kinds::kind_label(kinds::kind_of(tree.name_lower(r.id))).to_string(),
        ));
    }
    let attrs: Vec<&str> = [
        (flags::HIDDEN, "hidden"),
        (flags::SYSTEM, "system"),
        (flags::COMPRESSED, "compressed"),
        (flags::SPARSE, "sparse"),
        (flags::REPARSE, "link"),
        (flags::OFFLINE, "online-only"),
    ]
    .into_iter()
    .filter(|(fl, _)| n.has(*fl))
    .map(|(_, s)| s)
    .collect();
    if !attrs.is_empty() {
        lines.push(fact("Attributes", attrs.join(", ")));
    }

    if n.is_dir() && n.files > 0 {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            " Biggest inside",
            muted.add_modifier(Modifier::BOLD),
        ));
        let mut kids: Vec<NodeId> = tree
            .children(r.id)
            .filter(|&c| !tree.get(c).has(flags::GONE))
            .collect();
        kids.sort_by_key(|&c| std::cmp::Reverse(tree.get(c).alloc));
        for c in kids.into_iter().take(6) {
            let cn = tree.get(c);
            let frac = cn.alloc as f64 / n.alloc.max(1) as f64;
            let mut name = tree.name(c).to_string();
            if cn.is_dir() {
                name.push('\\');
            }
            lines.push(Line::from(vec![
                Span::raw(format!(" {:>9} ", out::size(cn.alloc))),
                Span::styled(
                    out::bar(frac, 5),
                    Style::new().fg(t.file(kind_key(tree, c))),
                ),
                Span::raw(format!(" {}", fit_right(&name, w.saturating_sub(17)))),
            ]));
        }

        // What kinds of files fill it; cached until the tree changes.
        let key = (r.letter, r.id);
        let generation = ui.seen;
        let fresh = ui.types.get(&key).is_some_and(|(g, _)| *g == generation);
        if !fresh {
            ui.types
                .insert(key, (generation, kinds::breakdown(tree, r.id)));
        }
        let b = &ui.types[&key].1;
        lines.push(Line::raw(""));
        lines.push(Line::styled(" By type", muted.add_modifier(Modifier::BOLD)));
        for k in b.kinds.iter().take(5) {
            let frac = k.alloc as f64 / b.alloc.max(1) as f64;
            lines.push(Line::from(vec![
                Span::raw(format!(" {:>9} ", out::size(k.alloc))),
                Span::styled(out::bar(frac, 5), Style::new().fg(t.file(k.key))),
                Span::raw(format!(" {}", k.label)),
                Span::styled(
                    format!(
                        "  {}",
                        k.exts
                            .iter()
                            .take(3)
                            .map(|e| format!(".{e}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                    subtle,
                ),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// A box centred in the screen.
fn centred(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn draw_confirm(f: &mut ratatui::Frame, c: &Confirm, t: &Theme) {
    let total: u64 = c.targets.iter().filter_map(|(_, x)| x.alloc).sum();
    let mut lines = Vec::new();
    if !c.targets.is_empty() {
        lines.push(Line::styled(
            format!(
                "Move {} ({}) to the Recycle Bin?",
                if c.targets.len() == 1 {
                    "1 item".to_string()
                } else {
                    format!("{} items", c.targets.len())
                },
                out::size(total)
            ),
            Style::new().add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::raw(""));
        for (_, x) in c.targets.iter().take(8) {
            lines.push(Line::from(vec![
                Span::raw(format!("{:>9}  ", out::size(x.alloc.unwrap_or(0)))),
                Span::styled(fit_left(&x.path, 60), Style::new().fg(t.muted)),
            ]));
        }
        if c.targets.len() > 8 {
            lines.push(Line::styled(
                format!("… and {} more", c.targets.len() - 8),
                Style::new().fg(t.subtle),
            ));
        }
    }
    if !c.refused.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Left alone:",
            Style::new().fg(t.warn).add_modifier(Modifier::BOLD),
        ));
        for (p, why) in c.refused.iter().take(5) {
            lines.push(Line::from(vec![
                Span::styled(fit_left(p, 44), Style::new().fg(t.muted)),
                Span::styled(format!("  {why}"), Style::new().fg(t.subtle)),
            ]));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        if c.targets.is_empty() {
            "Any key to close"
        } else {
            "Enter or y: delete    any other key: cancel"
        },
        Style::new().fg(t.subtle),
    ));
    let area = centred(f.area(), 80, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(Style::new().fg(t.danger))
                .title(" Delete "),
        ),
        area,
    );
}

fn draw_help(f: &mut ratatui::Frame, t: &Theme) {
    let keys = [
        ("Type", "search (same syntax as bulk search --help)"),
        (
            "→ / ←",
            "into a folder / up, at the end / start of the search",
        ),
        ("↑ ↓ PgUp PgDn", "move"),
        ("Tab / Shift+Tab", "pick or unpick, and move"),
        ("Ctrl+A", "pick everything listed"),
        ("Enter", "print the picked paths (or this one) and quit"),
        ("Ctrl+O", "open"),
        ("Ctrl+E", "show in Explorer"),
        ("Ctrl+B", "show in the Bulk app"),
        ("Alt+C", "copy the paths"),
        ("Ctrl+D", "move to the Recycle Bin (asks first)"),
        ("Ctrl+R", "search this folder / the drive / all drives"),
        ("Ctrl+S", "sort by best match, size, newest, name"),
        ("Ctrl+P / Ctrl+N", "earlier / later searches"),
        ("Ctrl+U", "clear the search"),
        ("Esc", "clear the search, or quit"),
    ];
    let mut lines: Vec<Line> = keys
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(
                    format!("{k:>16}  "),
                    Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw(*v),
            ])
        })
        .collect();
    lines.push(Line::raw(""));
    for ex in [
        "kind:video size:>1GB      big videos",
        "node_modules type:dir     every node_modules folder",
        "ext:log older:30d         old logs",
        "Downloads/*.exe           installers in Downloads",
    ] {
        lines.push(Line::styled(format!("  {ex}"), Style::new().fg(t.muted)));
    }
    let area = centred(f.area(), 76, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .border_style(Style::new().fg(t.accent))
                .title(" bulk find "),
        ),
        area,
    );
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
