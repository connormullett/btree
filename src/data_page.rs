use std::convert::TryFrom;

use crate::{
    error::Error,
    page::Value,
    page_layout::{PAGE_SIZE, PTR_SIZE},
};

pub struct DataPage {
    data: Box<[u8; PAGE_SIZE]>,
}

impl DataPage {
    pub fn new(data: [u8; PAGE_SIZE]) -> DataPage {
        DataPage {
            data: Box::new(data),
        }
    }

    /// write_value_at_offset writes a given value (as BigEndian) at a certain offset
    /// overriding values at that offset.
    pub fn write_value_at_offset(&mut self, offset: usize, value: usize) -> Result<(), Error> {
        if offset > PAGE_SIZE - PTR_SIZE {
            return Err(Error::UnexpectedError);
        }
        let bytes = value.to_be_bytes();
        self.data[offset..offset + PTR_SIZE].clone_from_slice(&bytes);
        Ok(())
    }

    /// get_value_from_offset Fetches a value calculated as BigEndian, sized to usize.
    /// This function may error as the value might not fit into a usize.
    pub fn get_value_from_offset(&self, offset: usize) -> Result<usize, Error> {
        let bytes = &self.data[offset..offset + PTR_SIZE];
        let Value(res) = Value::try_from(bytes)?;
        Ok(res)
    }

    /// insert_bytes_at_offset pushes #size bytes from offset to end_offset
    /// inserts #size bytes from given slice.
    pub fn insert_bytes_at_offset(
        &mut self,
        bytes: &[u8],
        offset: usize,
        end_offset: usize,
        size: usize,
    ) -> Result<(), Error> {
        // This Should not occur - better verify.
        if end_offset + size > self.data.len() {
            return Err(Error::UnexpectedError);
        }
        for idx in (offset..=end_offset).rev() {
            self.data[idx + size] = self.data[idx]
        }
        self.data[offset..offset + size].clone_from_slice(bytes);
        Ok(())
    }

    /// write_bytes_at_offset write bytes at a certain offset overriding previous values.
    pub fn write_bytes_at_offset(
        &mut self,
        bytes: &[u8],
        offset: usize,
        size: usize,
    ) -> Result<(), Error> {
        self.data[offset..offset + size].clone_from_slice(bytes);
        Ok(())
    }

    /// get_ptr_from_offset Fetches a slice of bytes from certain offset and of certain size.
    pub fn get_at_offset(&self, offset: usize, size: usize) -> &[u8] {
        &self.data[offset..offset + size]
    }

    /// get_data returns the underlying array.
    pub fn get_data(&self) -> [u8; PAGE_SIZE] {
        *self.data
    }
}
