use byteorder::{BigEndian, ReadBytesExt};

use crate::data_page::DataPage;
use crate::error::Error;
use crate::node_type::{Key, KeyValuePair, NodeType, Offset};
use crate::page::Page;
use crate::page_layout::{
    FromByte, INTERNAL_NODE_HEADER_SIZE, INTERNAL_NODE_NUM_CHILDREN_OFFSET, IS_ROOT_OFFSET,
    KEY_SIZE, LEAF_NODE_DATA_PAGE_OFFSET, LEAF_NODE_DATA_PAGE_OFFSET_SIZE, LEAF_NODE_HEADER_SIZE,
    NODE_TYPE_OFFSET, PARENT_POINTER_OFFSET, PTR_SIZE, VALUE_SIZE,
};
use crate::pager::Pager;
use std::convert::TryFrom;
use std::mem::size_of;
use std::str;

/// Node represents a node in the BTree occupied by a single page in memory.
#[derive(Clone, Debug)]
pub struct Node {
    pub node_type: NodeType,
    pub is_root: bool,
    pub parent_offset: Option<Offset>,
}

// Node represents a node in the B-Tree.
impl Node {
    pub fn new(node_type: NodeType, is_root: bool, parent_offset: Option<Offset>) -> Node {
        Node {
            node_type,
            is_root,
            parent_offset,
        }
    }

    /// split creates a sibling node from a given node by splitting the node in two around a median.
    /// split will split the child at b leaving the [0, b-1] keys
    /// while moving the set of [b, 2b-1] keys to the sibling.
    pub fn split(&mut self, b: usize, pager: &mut Pager) -> Result<(Key, Node), Error> {
        match &mut self.node_type {
            NodeType::Internal(ref mut children, ref mut keys) => {
                // Populate siblings keys.
                let mut sibling_keys = keys.split_off(b - 1);
                // Pop median key - to be added to the parent..
                let median_key = sibling_keys.remove(0);
                // Populate siblings children.
                let sibling_children = children.split_off(b);
                Ok((
                    median_key,
                    Node::new(
                        NodeType::Internal(sibling_children, sibling_keys),
                        false,
                        self.parent_offset.clone(),
                    ),
                ))
            }
            NodeType::Leaf(offset, ref mut pairs) => {
                // Populate siblings pairs.
                let mut sibling_pairs = pairs.split_off(b);
                // Pop median key.
                let median_pair = pairs.get(b - 1).ok_or(Error::UnexpectedError)?.clone();
                // get data page as node
                let page = pager.get_page(&offset)?;
                let mut data_page = DataPage::try_from(page)?;
                // split data page and reset values for sibling
                let (left, right) = data_page.split(b);
                pager.write_page_at_offset(Page::try_from(&left)?, &offset)?;
                let sibling_offset = pager.write_page(Page::try_from(&right)?)?;

                // find minumum index
                let mut min = usize::MAX;
                for pair in sibling_pairs.iter() {
                    if pair.idx <= min {
                        min = pair.idx;
                    }
                }

                // subtract minimum from indexes in sibling values
                for pair in sibling_pairs.iter_mut() {
                    pair.idx -= min;
                }

                Ok((
                    Key(median_pair.key),
                    Node::new(
                        NodeType::Leaf(sibling_offset, sibling_pairs),
                        false,
                        self.parent_offset.clone(),
                    ),
                ))
            }
            NodeType::Unexpected => Err(Error::UnexpectedError),
        }
    }
}

/// Implement TryFrom<Page> for Node allowing for easier
/// deserialization of data from a Page.
impl TryFrom<Page> for Node {
    type Error = Error;
    fn try_from(page: Page) -> Result<Node, Error> {
        let raw = page.get_data();
        let node_type = NodeType::from(raw[NODE_TYPE_OFFSET]);
        let is_root = raw[IS_ROOT_OFFSET].from_byte();
        let parent_offset: Option<Offset>;
        if is_root {
            parent_offset = None;
        } else {
            parent_offset = Some(Offset(page.get_value_from_offset(PARENT_POINTER_OFFSET)?));
        }

        match node_type {
            NodeType::Internal(mut children, mut keys) => {
                let num_children = page.get_value_from_offset(INTERNAL_NODE_NUM_CHILDREN_OFFSET)?;
                let mut offset = INTERNAL_NODE_HEADER_SIZE;
                for _i in 1..=num_children {
                    let child_offset = page.get_value_from_offset(offset)?;
                    children.push(Offset(child_offset));
                    offset += PTR_SIZE;
                }

                // Number of keys is always one less than the number of children (i.e. branching factor)
                for _i in 1..num_children {
                    let key_raw = page.get_ptr_from_offset(offset, KEY_SIZE);
                    let key = match str::from_utf8(key_raw) {
                        Ok(key) => key,
                        Err(_) => return Err(Error::UTF8Error),
                    };
                    offset += KEY_SIZE;
                    // Trim leading or trailing zeros.
                    keys.push(Key(key.trim_matches(char::from(0)).to_string()));
                }
                Ok(Node::new(
                    NodeType::Internal(children, keys),
                    is_root,
                    parent_offset,
                ))
            }

            NodeType::Leaf(_, mut pairs) => {
                // data page offset
                let mut offset = LEAF_NODE_DATA_PAGE_OFFSET;
                let data_offset = Offset(page.get_value_from_offset(offset)?);

                offset += LEAF_NODE_DATA_PAGE_OFFSET_SIZE;
                // key value pairs
                let num_keys_val_pairs = page.get_value_from_offset(offset)?;
                offset = LEAF_NODE_HEADER_SIZE;

                for _i in 0..num_keys_val_pairs {
                    let key_raw = page.get_ptr_from_offset(offset, KEY_SIZE);
                    let key = match str::from_utf8(key_raw) {
                        Ok(key) => key,
                        Err(_) => return Err(Error::UTF8Error),
                    };
                    offset += KEY_SIZE;

                    let mut value_offset_raw = page.get_ptr_from_offset(offset, size_of::<usize>());
                    let value_offset = value_offset_raw.read_u64::<BigEndian>()? as usize;
                    offset += VALUE_SIZE;

                    // Trim leading or trailing zeros.
                    pairs.push(KeyValuePair::new(
                        key.trim_matches(char::from(0)).to_string(),
                        value_offset,
                    ))
                }
                Ok(Node::new(
                    NodeType::Leaf(data_offset, pairs),
                    is_root,
                    parent_offset,
                ))
            }

            NodeType::Unexpected => Err(Error::UnexpectedError),
        }
    }
}

////////////////////
///              ///
///  Unit Tests. ///
///              ///
////////////////////

#[cfg(test)]
mod tests {
    use crate::data_page::DataPage;
    use crate::error::Error;
    use crate::node::{
        Node, Page, INTERNAL_NODE_HEADER_SIZE, KEY_SIZE, LEAF_NODE_HEADER_SIZE, PTR_SIZE,
        VALUE_SIZE,
    };
    use crate::node_type::{Key, NodeType, Offset};
    use crate::page_layout::PAGE_SIZE;
    use crate::pager::Pager;
    use std::convert::TryFrom;
    use std::path::Path;

    #[test]
    fn page_to_node_works_for_leaf_node() -> Result<(), Error> {
        const DATA_LEN: usize = LEAF_NODE_HEADER_SIZE + KEY_SIZE + VALUE_SIZE;
        let page_data: [u8; DATA_LEN] = [
            0x01, // Is-Root byte.
            0x02, // Leaf Node type byte.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Parent offset.
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // DataPage offset.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // Number of Key-Value pairs.
            0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x00, 0x00, 0x00, 0x00, 0x00, // "hello"
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x77, 0x6f, 0x72, 0x6c, 0x64, 0x00, 0x00, 0x00, // "world"
        ];
        let junk: [u8; PAGE_SIZE - DATA_LEN] = [0x00; PAGE_SIZE - DATA_LEN];
        let mut page = [0x00; PAGE_SIZE];
        for (to, from) in page.iter_mut().zip(page_data.iter().chain(junk.iter())) {
            *to = *from
        }

        let node = Node::try_from(Page::new(page))?;

        assert_eq!(node.is_root, true);
        Ok(())
    }

    #[test]
    fn page_to_node_works_for_internal_node() -> Result<(), Error> {
        use crate::node_type::Key;
        const DATA_LEN: usize = INTERNAL_NODE_HEADER_SIZE + 3 * PTR_SIZE + 2 * KEY_SIZE;
        let page_data: [u8; DATA_LEN] = [
            0x01, // Is-Root byte.
            0x01, // Internal Node type byte.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Parent offset.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, // Number of children.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, // 4096  (2nd Page)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00, // 8192  (3rd Page)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x00, // 12288 (4th Page)
            0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x00, 0x00, 0x00, 0x00, 0x00, // "hello"
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x77, 0x6f, 0x72, 0x6c, 0x64, 0x00, 0x00, 0x00, 0x00, 0x00, // "world"
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
        ];
        let junk: [u8; PAGE_SIZE - DATA_LEN] = [0x00; PAGE_SIZE - DATA_LEN];

        // Concatenate the two arrays; page_data and junk.
        let mut page = [0x00; PAGE_SIZE];
        for (to, from) in page.iter_mut().zip(page_data.iter().chain(junk.iter())) {
            *to = *from
        }

        let node = Node::try_from(Page::new(page))?;

        if let NodeType::Internal(_, keys) = node.node_type {
            assert_eq!(keys.len(), 2);

            let Key(first_key) = match keys.get(0) {
                Some(key) => key,
                None => return Err(Error::UnexpectedError),
            };
            assert_eq!(first_key, "hello");

            let Key(second_key) = match keys.get(1) {
                Some(key) => key,
                None => return Err(Error::UnexpectedError),
            };
            assert_eq!(second_key, "world");
            return Ok(());
        }

        Err(Error::UnexpectedError)
    }

    #[test]
    fn split_leaf_works() -> Result<(), Error> {
        use crate::node::Node;
        use crate::node_type::KeyValuePair;
        let mut pager = Pager::new(&Path::new("/tmp/pager"))?;
        let mut data_page = DataPage::new();
        data_page.insert("bar".to_string());
        data_page.insert("foo".to_string());
        data_page.insert("zap".to_string());
        pager.write_page(Page::try_from(&data_page)?)?;

        let mut node = Node::new(
            NodeType::Leaf(
                Offset(0),
                vec![
                    KeyValuePair::new("foo".to_string(), 0),
                    KeyValuePair::new("lebron".to_string(), 1),
                    KeyValuePair::new("ariana".to_string(), 2),
                ],
            ),
            true,
            None,
        );
        let offset = pager.write_page(Page::try_from(&node)?)?;
        assert_eq!(offset, Offset(4096));

        let (median, sibling) = node.split(2, &mut pager)?;
        assert_eq!(median, Key("lebron".to_string()));
        assert_eq!(
            node.node_type,
            NodeType::Leaf(
                Offset(0),
                vec![
                    KeyValuePair {
                        key: "foo".to_string(),
                        idx: 0
                    },
                    KeyValuePair {
                        key: "lebron".to_string(),
                        idx: 1
                    }
                ]
            )
        );

        let sibling_key_values = match sibling.node_type {
            NodeType::Leaf(_, key_values) => key_values,
            _ => panic!("expected leaf node"),
        };

        assert_eq!(
            sibling_key_values,
            vec![KeyValuePair {
                key: "ariana".to_string(),
                idx: 0
            }]
        );
        Ok(())
    }

    #[test]
    fn split_internal_works() -> Result<(), Error> {
        use crate::node::Node;
        use crate::node_type::NodeType;
        use crate::node_type::{Key, Offset};
        use crate::page_layout::PAGE_SIZE;
        let mut pager = Pager::new(&Path::new("/tmp/pager")).unwrap();
        let mut node = Node::new(
            NodeType::Internal(
                vec![
                    Offset(PAGE_SIZE),
                    Offset(PAGE_SIZE * 2),
                    Offset(PAGE_SIZE * 3),
                    Offset(PAGE_SIZE * 4),
                ],
                vec![
                    Key("foo bar".to_string()),
                    Key("lebron".to_string()),
                    Key("ariana".to_string()),
                ],
            ),
            true,
            None,
        );

        let (median, sibling) = node.split(2, &mut pager)?;
        assert_eq!(median, Key("lebron".to_string()));
        assert_eq!(
            node.node_type,
            NodeType::Internal(
                vec![Offset(PAGE_SIZE), Offset(PAGE_SIZE * 2)],
                vec![Key("foo bar".to_string())]
            )
        );
        assert_eq!(
            sibling.node_type,
            NodeType::Internal(
                vec![Offset(PAGE_SIZE * 3), Offset(PAGE_SIZE * 4)],
                vec![Key("ariana".to_string())]
            )
        );
        Ok(())
    }
}
