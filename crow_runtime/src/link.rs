//! A record's link to a record of its kind — a tree's child, a chain's
//! next — held shared: a copy of a record is a count, and a write through a
//! link copies the record only where another value still holds it (spec,
//! Records: a record is a value). The links had been boxes, so reading
//! `t.left` copied the whole subtree below it: a persistent tree's split
//! copied every node it passed, and a treap ran a thousand times slower
//! than Rust's.
//!
//! One count and no weak one (triomphe): a node of two links and a flag
//! fits the 32-byte slot its box took.

/// A link: the record it holds, shared by every value that holds it.
pub type Link<T> = triomphe::Arc<T>;

/// A link to a record made here.
#[inline]
pub fn new<T>(record: T) -> Link<T> {
    triomphe::Arc::new(record)
}

/// The record a link holds, to be written: copied first where another
/// value holds it too.
#[inline]
pub fn make_mut<T: Clone>(link: &mut Link<T>) -> &mut T {
    triomphe::Arc::make_mut(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Node {
        key: i64,
        left: Option<Link<Node>>,
    }

    #[test]
    fn a_copy_shares_the_record_and_a_write_copies_it() {
        let leaf = Node { key: 1, left: None };
        let mut a = new(Node { key: 2, left: Some(new(leaf)) });
        let b = a.clone();
        assert!(triomphe::Arc::ptr_eq(&a, &b));
        make_mut(&mut a).key = 3;
        assert_eq!(a.key, 3);
        assert_eq!(b.key, 2);
        // the child below is still one record, held by both
        let (Some(x), Some(y)) = (&a.left, &b.left) else { panic!("a child was lost") };
        assert!(triomphe::Arc::ptr_eq(x, y));
    }

    #[test]
    fn a_link_nothing_else_holds_is_written_where_it_lies() {
        let mut a = new(Node { key: 1, left: None });
        let before = &*a as *const Node;
        make_mut(&mut a).key = 5;
        assert_eq!(&*a as *const Node, before);
    }

    #[test]
    fn a_node_of_two_links_and_a_flag_takes_32_bytes() {
        #[allow(dead_code)]
        struct Tree {
            left: Option<Link<Tree>>,
            right: Option<Link<Tree>>,
            leaf: bool,
        }
        // the count beside the record: 8 bytes, then the record's 24
        assert_eq!(std::mem::size_of::<Option<Link<Tree>>>(), 8);
        assert_eq!(std::mem::size_of::<Tree>(), 24);
    }
}
