use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

use crate::{
    data_page::DataPage, error::Error, node_type::Offset, page::Page, page_layout::PAGE_SIZE,
};

// leaf nodes will contain the offset of where their page lives
// should be able to sort keys and split pages (see TryFrom impls)
// might require new data type
pub struct DataPager {
    file: File,
    cursor: usize,
}

impl DataPager {
    pub fn new(path: &Path) -> Result<DataPager, Error> {
        let fd = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(path)?;

        Ok(DataPager {
            file: fd,
            cursor: 0,
        })
    }

    pub fn get_page(&mut self, offset: &Offset) -> Result<Page, Error> {
        let mut page: [u8; PAGE_SIZE] = [0x00; PAGE_SIZE];
        self.file.seek(SeekFrom::Start(offset.0 as u64))?;
        self.file.read_exact(&mut page)?;
        Ok(Page::new(page))
    }

    pub fn write_page(&mut self, page: DataPage) -> Result<Offset, Error> {
        self.file.seek(SeekFrom::Start(self.cursor as u64))?;
        self.file.write_all(&page.get_data())?;
        let res = Offset(self.cursor);
        self.cursor += PAGE_SIZE;
        Ok(res)
    }

    pub fn write_page_at_offset(&mut self, page: DataPage, offset: &Offset) -> Result<(), Error> {
        self.file.seek(SeekFrom::Start(offset.0 as u64))?;
        self.file.write_all(&page.get_data())?;
        Ok(())
    }
}
