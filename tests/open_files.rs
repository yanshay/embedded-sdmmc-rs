//! File opening related tests

use std::{cell::Cell, rc::Rc};

use embedded_sdmmc::blocking::{
    Block, BlockCount, BlockDevice, BlockIdx, Error, Mode, VolumeIdx, VolumeManager,
};

mod utils;

#[derive(Debug)]
#[allow(dead_code)]
enum FailingBlockError {
    Inner(utils::Error),
    InjectedRead,
}

struct OneShotReadFailureBlockDevice<D> {
    inner: D,
    read_failures_remaining: Rc<Cell<u32>>,
}

impl<D> BlockDevice for OneShotReadFailureBlockDevice<D>
where
    D: BlockDevice<Error = utils::Error>,
{
    type Error = FailingBlockError;

    fn read(&self, blocks: &mut [Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        let failures_remaining = self.read_failures_remaining.get();
        if failures_remaining > 0 {
            self.read_failures_remaining.set(failures_remaining - 1);
            return Err(FailingBlockError::InjectedRead);
        }

        self.inner
            .read(blocks, start_block_idx)
            .map_err(FailingBlockError::Inner)
    }

    fn write(&self, blocks: &[Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        self.inner
            .write(blocks, start_block_idx)
            .map_err(FailingBlockError::Inner)
    }

    fn num_blocks(&self) -> Result<BlockCount, Self::Error> {
        self.inner.num_blocks().map_err(FailingBlockError::Inner)
    }
}

#[test]
fn open_files() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let volume_mgr: VolumeManager<utils::RamDisk<Vec<u8>>, utils::TestTimeSource, 4, 2, 1> =
        VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr
        .open_raw_volume(VolumeIdx(0))
        .expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");

    // Open with string
    let f = volume_mgr
        .open_file_in_dir(root_dir, "README.TXT", Mode::ReadWriteTruncate)
        .expect("open file");

    assert!(matches!(
        volume_mgr.open_file_in_dir(root_dir, "README.TXT", Mode::ReadOnly),
        Err(Error::FileAlreadyOpen)
    ));

    volume_mgr.close_file(f).expect("close file");

    // Open with SFN

    let dir_entry = volume_mgr
        .find_directory_entry(root_dir, "README.TXT")
        .expect("find file");

    let f = volume_mgr
        .open_file_in_dir(root_dir, &dir_entry.name, Mode::ReadWriteCreateOrAppend)
        .expect("open file with dir entry");

    assert!(matches!(
        volume_mgr.open_file_in_dir(root_dir, &dir_entry.name, Mode::ReadOnly),
        Err(Error::FileAlreadyOpen)
    ));

    // Can still spot duplicates even if name given the other way around

    assert!(matches!(
        volume_mgr.open_file_in_dir(root_dir, "README.TXT", Mode::ReadOnly),
        Err(Error::FileAlreadyOpen)
    ));

    let f2 = volume_mgr
        .open_file_in_dir(root_dir, "64MB.DAT", Mode::ReadWriteTruncate)
        .expect("open file");

    // Hit file limit

    assert!(matches!(
        volume_mgr.open_file_in_dir(root_dir, "EMPTY.DAT", Mode::ReadOnly),
        Err(Error::TooManyOpenFiles)
    ));

    volume_mgr.close_file(f).expect("close file");
    volume_mgr.close_file(f2).expect("close file");

    // File not found

    assert!(matches!(
        volume_mgr.open_file_in_dir(root_dir, "README.TXS", Mode::ReadOnly),
        Err(Error::NotFound)
    ));

    // Create a new file
    let f3 = volume_mgr
        .open_file_in_dir(root_dir, "NEWFILE.DAT", Mode::ReadWriteCreate)
        .expect("open file");

    volume_mgr.write(f3, b"12345").expect("write to file");
    volume_mgr.write(f3, b"67890").expect("write to file");
    volume_mgr.close_file(f3).expect("close file");

    // Open our new file
    let f3 = volume_mgr
        .open_file_in_dir(root_dir, "NEWFILE.DAT", Mode::ReadOnly)
        .expect("open file");
    // Should have 10 bytes in it
    assert_eq!(volume_mgr.file_length(f3).expect("file length"), 10);
    volume_mgr.close_file(f3).expect("close file");

    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).expect("close volume");
}

#[test]
fn open_file_in_dir_create_propagates_lookup_error() {
    let time_source = utils::make_time_source();
    let read_failures_remaining = Rc::new(Cell::new(0));
    let disk = OneShotReadFailureBlockDevice {
        inner: utils::make_block_device(utils::DISK_SOURCE).unwrap(),
        read_failures_remaining: Rc::clone(&read_failures_remaining),
    };
    let volume_mgr: VolumeManager<_, utils::TestTimeSource, 4, 2, 1> =
        VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr
        .open_raw_volume(VolumeIdx(0))
        .expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");

    read_failures_remaining.set(1);
    assert!(matches!(
        volume_mgr.open_file_in_dir(root_dir, "NEWFILE.DAT", Mode::ReadWriteCreate),
        Err(Error::DeviceError(FailingBlockError::InjectedRead))
    ));

    assert!(matches!(
        volume_mgr.open_file_in_dir(root_dir, "NEWFILE.DAT", Mode::ReadOnly),
        Err(Error::NotFound)
    ));
}

#[test]
fn open_non_raw() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let volume_mgr: VolumeManager<utils::RamDisk<Vec<u8>>, utils::TestTimeSource, 4, 2, 1> =
        VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr.open_volume(VolumeIdx(0)).expect("open volume");
    let root_dir = volume.open_root_dir().expect("open root dir");
    let f = root_dir
        .open_file_in_dir("README.TXT", Mode::ReadOnly)
        .expect("open file");

    let mut buffer = [0u8; 512];
    let len = f.read(&mut buffer).expect("read from file");
    // See directory listing in utils.rs, to see that README.TXT is 258 bytes long
    assert_eq!(len, 258);
    assert_eq!(f.length(), 258);
    f.seek_from_current(0).unwrap();
    assert!(f.is_eof());
    assert_eq!(f.offset(), 258);
    assert!(matches!(f.seek_from_current(1), Err(Error::InvalidOffset)));
    f.seek_from_current(-258).unwrap();
    assert!(!f.is_eof());
    assert_eq!(f.offset(), 0);
    f.seek_from_current(10).unwrap();
    assert!(!f.is_eof());
    assert_eq!(f.offset(), 10);
    f.seek_from_end(0).unwrap();
    assert!(f.is_eof());
    assert_eq!(f.offset(), 258);
    assert!(matches!(
        f.seek_from_current(-259),
        Err(Error::InvalidOffset)
    ));
    f.seek_from_start(25).unwrap();
    assert!(!f.is_eof());
    assert_eq!(f.offset(), 25);

    drop(f);

    let Err(Error::FileAlreadyExists) =
        root_dir.open_file_in_dir("README.TXT", Mode::ReadWriteCreate)
    else {
        panic!("Expected to file to exist");
    };
}

// ****************************************************************************
//
// End Of File
//
// ****************************************************************************
