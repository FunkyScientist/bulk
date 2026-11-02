use super::*;
use bulk_core::snapshot::Meta;
use bulk_core::volume::{DriveKind, Volume};
use ratatui::backend::TestBackend;

/// Q:\a\{big.mp4, small.txt, sub\deep.log}, Q:\b.zip
fn shared() -> Arc<Shared> {
    let mut t = Tree::new("Q:");
    let a = t.add(t.root, "a", flags::DIR, 0, 0, 0, 0);
    t.add(a, "big.mp4", 0, 9000, 9000, 100, 0);
    t.add(a, "small.txt", 0, 10, 8192, 100, 0);
    let sub = t.add(a, "sub", flags::DIR, 0, 0, 0, 0);
    t.add(sub, "deep.log", 0, 500, 4096, 100, 0);
    t.add(t.root, "b.zip", 0, 100, 4096, 100, 0);
    t.aggregate();
    let vol = Volume {
        letter: 'Q',
        root: "Q:\\".into(),
        label: String::new(),
        fs: "NTFS".into(),
        serial: 1,
        kind: DriveKind::Fixed,
        total: 0,
        free: 0,
        cluster: 4096,
    };
    let meta = Meta {
        letter: 'Q',
        serial: 1,
        label: String::new(),
        taken: 0,
        method: "walk".into(),
        files: 4,
        alloc: 0,
        size: 0,
        journal: None,
    };
    Arc::new(Shared {
        drives: Mutex::new(vec![Drive {
            l: Loaded {
                vol,
                meta,
                tree: t,
                fresh: Fresh::Snapshot,
                changed: false,
            },
            index: None,
            watched: false,
        }]),
        generation: AtomicU64::new(0),
        status: Arc::new(Mutex::new(String::new())),
        stop: AtomicBool::new(false),
    })
}

fn ui(scope: Scope) -> Ui {
    Ui {
        input: String::new(),
        cur: 0,
        scope: scope.clone(),
        home: scope,
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
        seen: 0,
        history: Vec::new(),
        hist_pos: None,
        list_area: Rect::new(0, 2, 80, 20),
        types: HashMap::new(),
    }
}

fn press(ui: &mut Ui, s: &Shared, code: KeyCode, mods: KeyModifiers) -> Option<Exit> {
    let (tx, _rx) = mpsc::channel();
    let r = key(ui, KeyEvent::new(code, mods), s, &tx).unwrap();
    refresh(ui, &s.drives.lock().unwrap());
    r
}

fn names(ui: &Ui, s: &Shared) -> Vec<String> {
    let d = s.drives.lock().unwrap();
    ui.rows
        .iter()
        .map(|r| d[0].l.tree.name(r.id).to_string())
        .collect()
}

fn screen(ui: &mut Ui, s: &Shared) -> String {
    let mut term = Terminal::new(TestBackend::new(130, 24)).unwrap();
    let d = s.drives.lock().unwrap();
    term.draw(|f| draw(f, ui, &d, &Theme::load(), "")).unwrap();
    let buf = term.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn browse_into_and_out_of_folders() {
    let s = shared();
    let mut u = ui(Scope::Folder('Q', "Q:\\a".into()));
    refresh(&mut u, &s.drives.lock().unwrap());
    assert_eq!(names(&u, &s), ["big.mp4", "small.txt", "sub"]);

    u.sel = 2;
    press(&mut u, &s, KeyCode::Right, KeyModifiers::NONE);
    assert!(u.scope == Scope::Folder('Q', "Q:\\a\\sub".into()));
    assert_eq!(names(&u, &s), ["deep.log"]);

    press(&mut u, &s, KeyCode::Left, KeyModifiers::NONE);
    assert!(u.scope == Scope::Folder('Q', "Q:\\a".into()));
    assert_eq!(
        names(&u, &s)[u.sel],
        "sub",
        "lands on the folder it came from"
    );

    press(&mut u, &s, KeyCode::Left, KeyModifiers::NONE);
    assert!(u.scope == Scope::Drive('Q'));
    assert_eq!(names(&u, &s)[u.sel], "a");
}

#[test]
fn search_pick_and_print() {
    let s = shared();
    let mut u = ui(Scope::Drive('Q'));
    for c in "ext:mp4|log".chars() {
        press(&mut u, &s, KeyCode::Char(c), KeyModifiers::NONE);
    }
    assert_eq!(names(&u, &s), ["big.mp4", "deep.log"]);

    // Left mid-search edits the text instead of going up.
    press(&mut u, &s, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(u.cur, u.input.chars().count() - 1);
    assert!(u.scope == Scope::Drive('Q'));

    press(&mut u, &s, KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert_eq!(u.picked.len(), 2);
    press(&mut u, &s, KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert!(u.picked.is_empty());

    press(&mut u, &s, KeyCode::Down, KeyModifiers::NONE);
    press(&mut u, &s, KeyCode::BackTab, KeyModifiers::NONE);
    assert_eq!(u.picked.len(), 1);
    let Some(Exit::Print(rows)) = press(&mut u, &s, KeyCode::Enter, KeyModifiers::NONE) else {
        panic!("Enter should print");
    };
    let d = s.drives.lock().unwrap();
    assert_eq!(d[0].l.tree.path(rows[0].id), "Q:\\a\\sub\\deep.log");
}

#[test]
fn scope_sort_and_escape() {
    let s = shared();
    let mut u = ui(Scope::Folder('Q', "Q:\\a".into()));
    set_input(&mut u, "zip".into());
    refresh(&mut u, &s.drives.lock().unwrap());
    assert!(u.rows.is_empty(), "b.zip is outside Q:\\a");
    press(&mut u, &s, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert_eq!(names(&u, &s), ["b.zip"]);

    set_input(&mut u, String::new());
    press(&mut u, &s, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert_eq!(SORTS[u.sort].1, "size");

    set_input(&mut u, "x".into());
    assert!(press(&mut u, &s, KeyCode::Esc, KeyModifiers::NONE).is_none());
    assert!(u.input.is_empty());
    assert!(matches!(
        press(&mut u, &s, KeyCode::Esc, KeyModifiers::NONE),
        Some(Exit::Quit)
    ));
}

#[test]
fn delete_asks_first() {
    let s = shared();
    let mut u = ui(Scope::Folder('Q', "Q:\\a".into()));
    refresh(&mut u, &s.drives.lock().unwrap());
    press(&mut u, &s, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut u, &s, KeyCode::Char('d'), KeyModifiers::CONTROL);
    let c = u.confirm.as_ref().expect("asks before deleting");
    assert_eq!(c.targets.len(), 1);
    assert!(screen(&mut u, &s).contains("to the Recycle Bin?"));
    // Anything but Enter or y cancels, and nothing is deleted.
    press(&mut u, &s, KeyCode::Char('n'), KeyModifiers::NONE);
    assert!(u.confirm.is_none());
    assert_eq!(names(&u, &s).len(), 3);
}

#[test]
fn draws_rows_preview_and_help() {
    let s = shared();
    let mut u = ui(Scope::Folder('Q', "Q:\\a".into()));
    refresh(&mut u, &s.drives.lock().unwrap());
    let text = screen(&mut u, &s);
    assert!(text.contains("big.mp4"), "{text}");
    assert!(text.contains("On disk"), "{text}");
    assert!(text.contains("3 items"), "{text}");
    u.help = true;
    assert!(screen(&mut u, &s).contains("pick or unpick"));
}
