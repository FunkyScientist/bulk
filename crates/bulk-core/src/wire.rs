//! Binary encoding of a `Tree`, shared by the helper pipe and snapshot files.

use std::io::{Read, Write};

use crate::tree::{Node, Tree};

const MAGIC: &[u8; 4] = b"BLKT";
const VERSION: u32 = 1;

/// Exact number of bytes `write_tree` produces.
pub fn encoded_len(tree: &Tree) -> u64 {
    let (nodes, names) = tree.parts();
    28 + size_of_val(nodes) as u64 + names.len() as u64
}

pub fn write_tree(w: &mut impl Write, tree: &Tree) -> std::io::Result<()> {
    let (nodes, names) = tree.parts();
    w.write_all(MAGIC)?;
    w.write_all(&VERSION.to_le_bytes())?;
    w.write_all(&tree.root.to_le_bytes())?;
    w.write_all(&(nodes.len() as u64).to_le_bytes())?;
    w.write_all(&(names.len() as u64).to_le_bytes())?;
    w.write_all(node_bytes(nodes))?;
    w.write_all(names)?;
    Ok(())
}

pub fn read_tree(r: &mut impl Read) -> std::io::Result<Tree> {
    let mut head = [0u8; 28];
    r.read_exact(&mut head)?;
    if &head[0..4] != MAGIC || u32::from_le_bytes(head[4..8].try_into().unwrap()) != VERSION {
        return Err(invalid("unrecognised tree data"));
    }
    let root = u32::from_le_bytes(head[8..12].try_into().unwrap());
    let count = u64::from_le_bytes(head[12..20].try_into().unwrap()) as usize;
    let name_len = u64::from_le_bytes(head[20..28].try_into().unwrap()) as usize;
    if count == 0 || root as usize >= count || count > 1 << 31 || name_len > 1 << 34 {
        return Err(invalid("corrupt tree header"));
    }

    let mut nodes = vec![Node::default(); count];
    r.read_exact(node_bytes_mut(&mut nodes))?;
    let mut names = vec![0u8; name_len];
    r.read_exact(&mut names)?;

    // Validate everything `Tree` relies on so later accesses can't go out of bounds.
    std::str::from_utf8(&names).map_err(|_| invalid("names are not utf-8"))?;
    for n in &nodes {
        let link_ok = |id: u32| id == crate::tree::NONE || (id as usize) < count;
        if !link_ok(n.parent) || !link_ok(n.first_child) || !link_ok(n.next_sibling) {
            return Err(invalid("corrupt node link"));
        }
        let end = n.name_off as usize + n.name_len as usize;
        if end > name_len
            || !std::str::from_utf8(&names[n.name_off as usize..end]).is_ok()
        {
            return Err(invalid("corrupt node name"));
        }
    }
    Ok(Tree::from_parts(nodes, names, root))
}

fn invalid(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
}

fn node_bytes(nodes: &[Node]) -> &[u8] {
    // `Node` is `repr(C)` plain data with no padding.
    unsafe { std::slice::from_raw_parts(nodes.as_ptr() as *const u8, size_of_val(nodes)) }
}

fn node_bytes_mut(nodes: &mut [Node]) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut(nodes.as_mut_ptr() as *mut u8, size_of_val(nodes)) }
}

const _: () = assert!(size_of::<Node>() == 56);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::flags;

    #[test]
    fn roundtrip() {
        let mut t = Tree::new("C:");
        let d = t.add(t.root, "dir", flags::DIR, 0, 0, 1, 7);
        t.add(d, "file ü.txt", 0, 5, 4096, 2, 9);
        t.aggregate();
        let mut buf = Vec::new();
        write_tree(&mut buf, &t).unwrap();
        let back = read_tree(&mut buf.as_slice()).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back.get(back.root).size, 5);
        let f = back.find_path("C:\\dir\\file ü.txt").unwrap();
        assert_eq!(back.get(f).frn, 9);

        buf[30] ^= 0xFF; // break a node link
        assert!(read_tree(&mut buf.as_slice()).is_err());
    }
}
