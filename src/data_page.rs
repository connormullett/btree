use std::convert::TryFrom;

use crate::{error::Error, page::Page};

#[derive(Clone, Debug, Default)]
pub struct DataPage {
    pub values: Vec<String>,
}

impl DataPage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn _split(&self) -> Self {
        // try a sync strategy with leaf node
        todo!()
    }
}

impl TryFrom<Page> for DataPage {
    type Error = Error;

    fn try_from(page: Page) -> Result<Self, Self::Error> {
        let raw = page.get_data();
        let mut values = vec![];
        let num_values = raw[0];
        let mut offset = 1;
        for _ in 0..num_values {
            let len_value = raw[offset] as usize;
            offset += 1;
            let raw_value = &raw[offset..offset + len_value];
            let value = std::str::from_utf8(raw_value).map_err(|_| Error::UnexpectedError)?;
            values.push(value.to_string());
            offset += len_value;
        }

        Ok(Self { values })
    }
}
