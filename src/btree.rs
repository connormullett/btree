use crate::data_page::{self, DataPage};
use crate::error::Error;
use crate::node::Node;
use crate::node_type::{Key, KeyValuePair, NodeType, Offset};
use crate::page::Page;
use crate::pager::Pager;
use crate::wal::Wal;
use std::cmp;
use std::convert::TryFrom;
use std::path::Path;

/// B+Tree properties.
pub const MAX_BRANCHING_FACTOR: usize = 200;
pub const NODE_KEYS_LIMIT: usize = MAX_BRANCHING_FACTOR - 1;

/// BTree struct represents an on-disk B+tree.
/// Each node is persisted in the table file, the leaf nodes contain the values.
pub struct BTree {
    pager: Pager,
    b: usize,
    wal: Wal,
}

/// BtreeBuilder is a Builder for the BTree struct.
pub struct BTreeBuilder {
    /// Path to the tree file, db index.
    path: &'static Path,
    /// The BTree parameter, an inner node contains no more than 2*b-1 keys and no less than b-1 keys
    /// and no more than 2*b children and no less than b children.
    b: usize,
}

impl BTreeBuilder {
    pub fn new() -> BTreeBuilder {
        BTreeBuilder {
            path: Path::new(""),
            b: 0,
        }
    }

    pub fn path(mut self, path: &'static Path) -> BTreeBuilder {
        self.path = path;
        self
    }

    pub fn b_parameter(mut self, b: usize) -> BTreeBuilder {
        self.b = b;
        self
    }

    pub fn build(&self) -> Result<BTree, Error> {
        if self.path.to_string_lossy() == "" {
            return Err(Error::UnexpectedError);
        }
        if self.b == 0 {
            return Err(Error::UnexpectedError);
        }

        let mut pager = Pager::new(self.path)?;

        let data_page = DataPage::new();
        let root_page_offset = pager.write_page(Page::try_from(&data_page)?)?;

        let root = Node::new(NodeType::Leaf(root_page_offset, vec![]), true, None);
        let root_offset = pager.write_page(Page::try_from(&root)?)?;

        let parent_directory = self.path.parent().unwrap_or_else(|| Path::new("/tmp"));
        let mut wal = Wal::new(parent_directory.to_path_buf())?;
        wal.set_root(root_offset)?;

        Ok(BTree {
            pager,
            b: self.b,
            wal,
        })
    }
}

impl Default for BTreeBuilder {
    // A default BTreeBuilder provides a builder with:
    // - b parameter set to 200
    // - path set to '/tmp/db'.
    fn default() -> Self {
        BTreeBuilder::new()
            .b_parameter(200)
            .path(Path::new("/tmp/db"))
    }
}

impl BTree {
    fn is_node_full(&self, node: &Node) -> Result<bool, Error> {
        match &node.node_type {
            NodeType::Leaf(_, pairs) => Ok(pairs.len() == (2 * self.b - 1)),
            NodeType::Internal(_, keys) => Ok(keys.len() == (2 * self.b - 1)),
            NodeType::Unexpected => Err(Error::UnexpectedError),
        }
    }

    fn is_node_underflow(&self, node: &Node) -> Result<bool, Error> {
        match &node.node_type {
            // A root cannot really be "underflowing" as it can contain less than b-1 keys / pointers.
            NodeType::Leaf(_, pairs) => Ok(pairs.len() < self.b - 1 && !node.is_root),
            NodeType::Internal(_, keys) => Ok(keys.len() < self.b - 1 && !node.is_root),
            NodeType::Unexpected => Err(Error::UnexpectedError),
        }
    }

    /// insert a key value pair possibly splitting nodes along the way.
    pub fn insert(&mut self, key: String, value: String) -> Result<(), Error> {
        let root_offset = self.wal.get_root()?;
        let root_page = self.pager.get_page(&root_offset)?;
        let new_root_offset: Offset;
        let mut new_root: Node;
        let mut root = Node::try_from(root_page)?;
        if self.is_node_full(&root)? {
            // split the root creating a new root and child nodes along the way.
            new_root = Node::new(NodeType::Internal(vec![], vec![]), true, None);
            // write the new root to disk to aquire an offset for the new root.
            new_root_offset = self.pager.write_page(Page::try_from(&new_root)?)?;
            // set the old roots parent to the new root.
            root.parent_offset = Some(new_root_offset.clone());
            root.is_root = false;
            // split the old root.
            let (median, sibling) = root.split(self.b, &mut self.pager)?;

            // write the old root with its new data to disk in a *new* location.
            let old_root_offset = self.pager.write_page(Page::try_from(&root)?)?;
            // write the newly created sibling to disk.
            let sibling_offset = self.pager.write_page(Page::try_from(&sibling)?)?;
            // update the new root with its children and key.
            new_root.node_type =
                NodeType::Internal(vec![old_root_offset, sibling_offset], vec![median]);
            // write the new_root to disk.
            self.pager
                .write_page_at_offset(Page::try_from(&new_root)?, &new_root_offset)?;
        } else {
            new_root = root.clone();
            new_root_offset = self.pager.write_page(Page::try_from(&new_root)?)?;
        }
        // continue recursively.
        self.insert_non_full(&mut new_root, new_root_offset.clone(), key, value)?;
        // finish by setting the root to its new copy.
        self.wal.set_root(new_root_offset)
    }

    /// insert_non_full (recursively) finds a node rooted at a given non-full node.
    /// to insert a given key-value pair. Here we assume the node is
    /// already a copy of an existing node in a copy-on-write root to node traversal.
    fn insert_non_full(
        &mut self,
        node: &mut Node,
        node_offset: Offset,
        key: String,
        value: String,
    ) -> Result<(), Error> {
        match &mut node.node_type {
            NodeType::Leaf(ref mut data_offset, ref mut pairs) => {
                let mut kv = KeyValuePair { key, idx: 0 };
                let idx = pairs.binary_search(&kv).unwrap_or_else(|x| x);

                let page = self.pager.get_page(&data_offset)?;
                let mut data_page = DataPage::try_from(page)?;
                let data_idx = data_page.insert(value);
                kv.idx = data_idx;

                pairs.insert(idx, kv);

                let offset = self.pager.write_page(Page::try_from(&data_page)?)?;
                *data_offset = offset;
                self.pager
                    .write_page_at_offset(Page::try_from(&*node)?, &node_offset)
            }
            NodeType::Internal(ref mut children, ref mut keys) => {
                let idx = keys.binary_search(&Key(key.clone())).unwrap_or_else(|x| x);
                let child_offset: Offset = children.get(idx).ok_or(Error::UnexpectedError)?.clone();
                let child_page = self.pager.get_page(&child_offset)?;
                let mut child = Node::try_from(child_page)?;
                // Copy each branching-node on the root-to-leaf walk.
                // write_page appends the given page to the db file thus creating a new node.
                let new_child_offset = self.pager.write_page(Page::try_from(&child)?)?;
                // Assign copied child at the proper place.
                children[idx] = new_child_offset.to_owned();
                if self.is_node_full(&child)? {
                    // split will split the child at b leaving the [0, b-1] keys
                    // while moving the set of [b, 2b-1] keys to the sibling.
                    let (median, mut sibling) = child.split(self.b, &mut self.pager)?;
                    self.pager
                        .write_page_at_offset(Page::try_from(&child)?, &new_child_offset)?;
                    // Write the newly created sibling to disk.
                    let sibling_offset = self.pager.write_page(Page::try_from(&sibling)?)?;

                    // Siblings keys are larger than the splitted child thus need to be inserted
                    // at the next index.
                    children.insert(idx + 1, sibling_offset.clone());
                    keys.insert(idx, median.clone());

                    // Write the parent page to disk.
                    self.pager
                        .write_page_at_offset(Page::try_from(&*node)?, &node_offset)?;
                    // Continue recursively.
                    if key <= median.0 {
                        self.insert_non_full(&mut child, new_child_offset, key, value)
                    } else {
                        self.insert_non_full(&mut sibling, sibling_offset, key, value)
                    }
                } else {
                    self.pager
                        .write_page_at_offset(Page::try_from(&*node)?, &node_offset)?;
                    self.insert_non_full(&mut child, new_child_offset, key, value)
                }
            }
            NodeType::Unexpected => Err(Error::UnexpectedError),
        }
    }

    /// search searches for a specific key in the BTree.
    pub fn search(&mut self, key: String) -> Result<String, Error> {
        let root_offset = self.wal.get_root()?;
        let root_page = self.pager.get_page(&root_offset)?;
        let root = Node::try_from(root_page)?;
        self.search_node(root, &key)
    }

    /// search_node recursively searches a sub tree rooted at node for a key.
    fn search_node(&mut self, node: Node, search: &str) -> Result<String, Error> {
        match node.node_type {
            NodeType::Internal(children, keys) => {
                let idx = keys
                    .binary_search(&Key(search.to_string()))
                    .unwrap_or_else(|x| x);
                // Retrieve child page from disk and deserialize.
                let child_offset = children.get(idx).ok_or(Error::UnexpectedError)?;
                let page = self.pager.get_page(child_offset)?;
                let child_node = Node::try_from(page)?;
                self.search_node(child_node, search)
            }
            NodeType::Leaf(offset, pairs) => {
                if let Ok(idx) =
                    pairs.binary_search_by_key(&search.to_string(), |pair| pair.key.clone())
                {
                    let value = pairs.get(idx).ok_or(Error::KeyNotFound)?;
                    let page = self.pager.get_page(&offset)?;
                    let data_page = DataPage::try_from(page)?;
                    let value = data_page.get(value.idx).ok_or(Error::UnexpectedError)?;
                    return Ok(value);
                }
                Err(Error::KeyNotFound)
            }
            NodeType::Unexpected => Err(Error::UnexpectedError),
        }
    }

    /// delete deletes a given key from the tree.
    pub fn delete(&mut self, key: Key) -> Result<(), Error> {
        let root_offset = self.wal.get_root()?;
        let root_page = self.pager.get_page(&root_offset)?;
        // Shadow the new root and rewrite it.
        let mut new_root = Node::try_from(root_page)?;
        let new_root_page = Page::try_from(&new_root)?;
        let new_root_offset = self.pager.write_page(new_root_page)?;
        self.delete_key_from_subtree(key, &mut new_root, &new_root_offset)?;
        self.wal.set_root(new_root_offset)
    }

    /// delete key from subtree recursively traverses a tree rooted at a node in certain offset
    /// until it finds the given key and delete the key-value pair. Here we assume the node is
    /// already a copy of an existing node in a copy-on-write root to node traversal.
    fn delete_key_from_subtree(
        &mut self,
        key: Key,
        node: &mut Node,
        node_offset: &Offset,
    ) -> Result<(), Error> {
        match &mut node.node_type {
            NodeType::Leaf(ref mut data_offset, ref mut pairs) => {
                let key_idx = pairs
                    .binary_search_by_key(&key, |kv| Key(kv.key.clone()))
                    .map_err(|_| Error::KeyNotFound)?;

                // remove key from data page
                let page = self.pager.get_page(&data_offset)?;
                let mut data_page = DataPage::try_from(page)?;
                data_page.values.remove(key_idx);

                let offset = self.pager.write_page(Page::try_from(&data_page)?)?;
                *data_offset = offset;

                pairs.remove(key_idx);
                self.pager
                    .write_page_at_offset(Page::try_from(&*node)?, node_offset)?;
                // Check for underflow - if it occures,
                // we need to merge with a sibling.
                // this can only occur if node is not the root (as it cannot "underflow").
                // continue recoursively up the tree.
                self.borrow_if_needed(node.to_owned(), &key)?;
            }
            NodeType::Internal(children, keys) => {
                let node_idx = keys.binary_search(&key).unwrap_or_else(|x| x);
                // Retrieve child page from disk and deserialize,
                // copy over the child page and continue recursively.
                let child_offset = children.get(node_idx).ok_or(Error::UnexpectedError)?;
                let child_page = self.pager.get_page(child_offset)?;
                let mut child_node = Node::try_from(child_page)?;
                // Fix the parent_offset as the child node is a child of a copied parent
                // in a copy-on-write root to leaf traversal.
                // This is important for the case of a node underflow which might require a leaf to root traversal.
                child_node.parent_offset = Some(node_offset.to_owned());
                let new_child_page = Page::try_from(&child_node)?;
                let new_child_offset = self.pager.write_page(new_child_page)?;
                // Assign the new pointer in the parent and continue reccoursively.
                children[node_idx] = new_child_offset.to_owned();
                self.pager
                    .write_page_at_offset(Page::try_from(&*node)?, node_offset)?;
                return self.delete_key_from_subtree(key, &mut child_node, &new_child_offset);
            }
            NodeType::Unexpected => return Err(Error::UnexpectedError),
        }
        Ok(())
    }

    /// borrow_if_needed checks the node for underflow (following a removal of a key),
    /// if it underflows it is merged with a sibling node, and than called recoursively
    /// up the tree. Since the downward root-to-leaf traversal was done using the copy-on-write
    /// technique we are ensured that any merges will only be reflected in the copied parent in the path.
    fn borrow_if_needed(&mut self, node: Node, key: &Key) -> Result<(), Error> {
        if self.is_node_underflow(&node)? {
            // Fetch the sibling from the parent -
            // This could be quicker if we implement sibling pointers.
            let parent_offset = node.parent_offset.clone().ok_or(Error::UnexpectedError)?;
            let parent_page = self.pager.get_page(&parent_offset)?;
            let mut parent_node = Node::try_from(parent_page)?;
            // The parent has to be an "internal" node.
            match parent_node.node_type {
                NodeType::Internal(ref mut children, ref mut keys) => {
                    let idx = keys.binary_search(key).unwrap_or_else(|x| x);
                    // The sibling is in idx +- 1 as the above index led
                    // the downward search to node.
                    let sibling_idx;
                    match idx > 0 {
                        false => sibling_idx = idx + 1,
                        true => sibling_idx = idx - 1,
                    }

                    let sibling_offset = children.get(sibling_idx).ok_or(Error::UnexpectedError)?;
                    let sibling_page = self.pager.get_page(sibling_offset)?;
                    let sibling = Node::try_from(sibling_page)?;
                    let merged_node = self.merge(node, sibling)?;
                    let merged_node_offset =
                        self.pager.write_page(Page::try_from(&merged_node)?)?;
                    let merged_node_idx = cmp::min(idx, sibling_idx);
                    // remove the old nodes.
                    children.remove(merged_node_idx);
                    // remove shifts nodes to the left.
                    children.remove(merged_node_idx);
                    // if the parent is the root, and there is a single child - the merged node -
                    // we can safely replace the root with the child.
                    if parent_node.is_root && children.is_empty() {
                        self.wal.set_root(merged_node_offset)?;
                        return Ok(());
                    }
                    // remove the keys that separated the two nodes from each other:
                    keys.remove(idx);
                    // write the new node in place.
                    children.insert(merged_node_idx, merged_node_offset);
                    // write the updated parent back to disk and continue up the tree.
                    self.pager
                        .write_page_at_offset(Page::try_from(&parent_node)?, &parent_offset)?;
                    return self.borrow_if_needed(parent_node, key);
                }
                _ => return Err(Error::UnexpectedError),
            }
        }
        Ok(())
    }

    // merges two *sibling* nodes, it assumes the following:
    // 1. the two nodes are of the same type.
    // 2. the two nodes do not accumulate to an overflow,
    // i.e. |first.keys| + |second.keys| <= [2*(b-1) for keys or 2*b for offsets].
    fn merge(&self, first: Node, second: Node) -> Result<Node, Error> {
        match first.node_type {
            NodeType::Leaf(first_offset, first_pairs) => {
                if let NodeType::Leaf(second_offset, second_pairs) = second.node_type {
                    let merged_pairs: Vec<KeyValuePair> = first_pairs
                        .into_iter()
                        .chain(second_pairs.into_iter())
                        .collect();
                    let new_offset = todo!();
                    let node_type = NodeType::Leaf(new_offset, merged_pairs);
                    Ok(Node::new(node_type, first.is_root, first.parent_offset))
                } else {
                    Err(Error::UnexpectedError)
                }
            }
            NodeType::Internal(first_offsets, first_keys) => {
                if let NodeType::Internal(second_offsets, second_keys) = second.node_type {
                    let merged_keys: Vec<Key> = first_keys
                        .into_iter()
                        .chain(second_keys.into_iter())
                        .collect();
                    let merged_offsets: Vec<Offset> = first_offsets
                        .into_iter()
                        .chain(second_offsets.into_iter())
                        .collect();
                    let node_type = NodeType::Internal(merged_offsets, merged_keys);
                    Ok(Node::new(node_type, first.is_root, first.parent_offset))
                } else {
                    Err(Error::UnexpectedError)
                }
            }
            NodeType::Unexpected => Err(Error::UnexpectedError),
        }
    }

    /// print_sub_tree is a helper function for recursively printing the nodes rooted at a node given by its offset.
    fn print_sub_tree(&mut self, prefix: String, offset: Offset) -> Result<(), Error> {
        println!("{}Node at offset: {}", prefix, offset.0);
        let curr_prefix = format!("{}|->", prefix);
        let page = self.pager.get_page(&offset)?;
        let node = Node::try_from(page)?;
        match node.node_type {
            NodeType::Internal(children, keys) => {
                println!("{}Keys: {:?}", curr_prefix, keys);
                println!("{}Children: {:?}", curr_prefix, children);
                let child_prefix = format!("{}   |  ", prefix);
                for child_offset in children {
                    self.print_sub_tree(child_prefix.clone(), child_offset)?;
                }
                Ok(())
            }
            NodeType::Leaf(data_offset, pairs) => {
                println!(
                    "{}DataOffset: {}, Key value pairs: {:?}",
                    curr_prefix, data_offset.0, pairs
                );
                Ok(())
            }
            NodeType::Unexpected => Err(Error::UnexpectedError),
        }
    }

    /// print is a helper for recursively printing the tree.
    pub fn print(&mut self) -> Result<(), Error> {
        println!();
        let root_offset = self.wal.get_root()?;
        self.print_sub_tree("".to_string(), root_offset)
    }
}

#[cfg(test)]
mod tests {
    use crate::error::Error;

    #[test]
    fn search_works() -> Result<(), Error> {
        use crate::btree::BTreeBuilder;
        use std::path::Path;

        let mut btree = BTreeBuilder::new()
            .path(Path::new("/tmp/db"))
            .b_parameter(2)
            .build()?;
        btree.insert("a".to_string(), "shalom".to_string())?;
        btree.insert("b".to_string(), "hello".to_string())?;
        btree.insert("c".to_string(), "marhaba".to_string())?;

        let mut v = btree.search("b".to_string())?;
        assert_eq!(v, "hello");

        v = btree.search("c".to_string())?;
        assert_eq!(v, "marhaba");

        Ok(())
    }

    #[test]
    fn insert_works() -> Result<(), Error> {
        use crate::btree::BTreeBuilder;
        use std::path::Path;

        let mut btree = BTreeBuilder::new()
            .path(Path::new("/tmp/db"))
            .b_parameter(2)
            .build()?;
        btree.insert("a".to_string(), "shalom".to_string())?;
        btree.insert("b".to_string(), "hello".to_string())?;
        btree.insert("c".to_string(), "marhaba".to_string())?;
        btree.insert("d".to_string(), "olah".to_string())?;
        btree.insert("e".to_string(), "salam".to_string())?;
        btree.insert("f".to_string(), "hallo".to_string())?;
        btree.insert("g".to_string(), "Konnichiwa".to_string())?;
        btree.insert("h".to_string(), "Ni hao".to_string())?;
        btree.insert("i".to_string(), "Ciao".to_string())?;

        let mut v = btree.search("a".to_string())?;
        assert_eq!(v, "shalom");

        v = btree.search("b".to_string())?;
        assert_eq!(v, "hello");

        v = btree.search("c".to_string())?;
        assert_eq!(v, "marhaba");

        v = btree.search("d".to_string())?;
        assert_eq!(v, "olah");

        v = btree.search("e".to_string())?;
        assert_eq!(v, "salam");

        v = btree.search("f".to_string())?;
        assert_eq!(v, "hallo");

        v = btree.search("g".to_string())?;
        assert_eq!(v, "Konnichiwa");

        v = btree.search("h".to_string())?;
        assert_eq!(v, "Ni hao");

        v = btree.search("i".to_string())?;
        assert_eq!(v, "Ciao");
        Ok(())
    }

    #[test]
    fn delete_works() -> Result<(), Error> {
        use crate::btree::BTreeBuilder;
        use crate::error::Error;
        use crate::node_type::Key;
        use std::path::Path;

        let mut btree = BTreeBuilder::new()
            .path(Path::new("/tmp/db"))
            .b_parameter(2)
            .build()?;
        btree.insert("d".to_string(), "olah".to_string())?;
        btree.insert("e".to_string(), "salam".to_string())?;
        btree.insert("f".to_string(), "hallo".to_string())?;
        btree.insert("a".to_string(), "shalom".to_string())?;
        btree.insert("b".to_string(), "hello".to_string())?;
        btree.insert("c".to_string(), "marhaba".to_string())?;

        let mut v = btree.search("c".to_string())?;
        assert_eq!(v, "marhaba");

        btree.delete(Key("c".to_string()))?;
        let mut res = btree.search("c".to_string());
        assert!(matches!(res, Err(Error::KeyNotFound)));

        v = btree.search("d".to_string())?;
        assert_eq!(v, "olah");

        btree.delete(Key("d".to_string()))?;
        res = btree.search("d".to_string());
        assert!(matches!(res, Err(Error::KeyNotFound)));

        btree.delete(Key("e".to_string()))?;
        res = btree.search("e".to_string());
        assert!(matches!(res, Err(Error::KeyNotFound)));

        btree.delete(Key("f".to_string()))?;
        res = btree.search("f".to_string());
        assert!(matches!(res, Err(Error::KeyNotFound)));

        Ok(())
    }
}
