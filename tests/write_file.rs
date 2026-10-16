//! File opening related tests

use std::{cell::RefCell, rc::Rc};

use embedded_sdmmc::blocking::{
    Block, BlockCount, BlockDevice, BlockIdx, Error, Mode, VolumeIdx, VolumeManager,
};

mod utils;

struct RecordingBlockDevice<D> {
    inner: D,
    writes: Rc<RefCell<Vec<(BlockIdx, usize)>>>,
}

impl<D> BlockDevice for RecordingBlockDevice<D>
where
    D: BlockDevice,
{
    type Error = D::Error;

    fn read(&self, blocks: &mut [Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        self.inner.read(blocks, start_block_idx)
    }

    fn write(&self, blocks: &[Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        self.writes
            .borrow_mut()
            .push((start_block_idx, blocks.len()));
        self.inner.write(blocks, start_block_idx)
    }

    fn num_blocks(&self) -> Result<BlockCount, Self::Error> {
        self.inner.num_blocks()
    }
}

fn make_recording_device() -> (
    RecordingBlockDevice<utils::RamDisk<Vec<u8>>>,
    Rc<RefCell<Vec<(BlockIdx, usize)>>>,
) {
    let writes = Rc::new(RefCell::new(Vec::new()));
    let inner = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    (
        RecordingBlockDevice {
            inner,
            writes: writes.clone(),
        },
        writes,
    )
}

#[test]
fn append_file() {
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

    // Should be enough to cause a few more clusters to be allocated
    let test_data = vec![0xCC; 1024 * 1024];
    volume_mgr.write(f, &test_data).expect("file write");

    let length = volume_mgr.file_length(f).expect("get length");
    assert_eq!(length, 1024 * 1024);

    let offset = volume_mgr.file_offset(f).expect("offset");
    assert_eq!(offset, 1024 * 1024);

    // Now wind it back 1 byte;
    volume_mgr.file_seek_from_current(f, -1).expect("Seeking");

    let offset = volume_mgr.file_offset(f).expect("offset");
    assert_eq!(offset, (1024 * 1024) - 1);

    // Write another megabyte, making `2 MiB - 1`
    volume_mgr.write(f, &test_data).expect("file write");

    let length = volume_mgr.file_length(f).expect("get length");
    assert_eq!(length, (1024 * 1024 * 2) - 1);

    volume_mgr.close_file(f).expect("close dir");

    // Now check the file length again

    let entry = volume_mgr
        .find_directory_entry(root_dir, "README.TXT")
        .expect("Find entry");
    assert_eq!(entry.size, (1024 * 1024 * 2) - 1);

    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).expect("close volume");
}

#[test]
fn flush_file() {
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

    // Write some data to the file
    let test_data = vec![0xCC; 64];
    volume_mgr.write(f, &test_data).expect("file write");

    // Check that the file length is zero in the directory entry, as we haven't
    // flushed yet
    let entry = volume_mgr
        .find_directory_entry(root_dir, "README.TXT")
        .expect("find entry");
    assert_eq!(entry.size, 0);

    volume_mgr.flush_file(f).expect("flush");

    // Now check the file length again after flushing
    let entry = volume_mgr
        .find_directory_entry(root_dir, "README.TXT")
        .expect("find entry");
    assert_eq!(entry.size, 64);

    // Flush more writes
    volume_mgr.write(f, &test_data).expect("file write");
    volume_mgr.write(f, &test_data).expect("file write");
    volume_mgr.flush_file(f).expect("flush");

    // Now check the file length again, again
    let entry = volume_mgr
        .find_directory_entry(root_dir, "README.TXT")
        .expect("find entry");
    assert_eq!(entry.size, 64 * 3);
}

#[test]
fn random_access_write_file() {
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

    let test_data = vec![0xCC; 1024];
    volume_mgr.write(f, &test_data).expect("file write");

    let length = volume_mgr.file_length(f).expect("get length");
    assert_eq!(length, 1024);

    for seek_offset in [100, 0] {
        let mut expected_buffer = [0u8; 4];

        // fetch some data at offset seek_offset
        volume_mgr
            .file_seek_from_start(f, seek_offset)
            .expect("Seeking");
        volume_mgr.read(f, &mut expected_buffer).expect("read file");

        // modify first byte
        expected_buffer[0] ^= 0xff;

        // write only first byte, expecting the rest to not change
        volume_mgr
            .file_seek_from_start(f, seek_offset)
            .expect("Seeking");
        volume_mgr
            .write(f, &expected_buffer[0..1])
            .expect("file write");
        volume_mgr.flush_file(f).expect("file flush");

        // read and verify
        volume_mgr
            .file_seek_from_start(f, seek_offset)
            .expect("file seek");
        let mut read_buffer = [0xffu8, 0xff, 0xff, 0xff];
        volume_mgr.read(f, &mut read_buffer).expect("file read");
        assert_eq!(
            read_buffer, expected_buffer,
            "mismatch seek+write at offset {seek_offset} from start"
        );
    }

    volume_mgr.close_file(f).expect("close file");
    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).expect("close volume");
}

#[test]
fn bulk_write_uses_multiblock_requests_and_supports_byte_tail() {
    let time_source = utils::make_time_source();
    let (disk, writes) = make_recording_device();
    let volume_mgr: VolumeManager<_, _, 4, 2, 1> =
        VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr
        .open_raw_volume(VolumeIdx(0))
        .expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");
    let file = volume_mgr
        .open_file_in_dir(root_dir, "EMPTY.DAT", Mode::ReadWriteTruncate)
        .expect("open file");

    writes.borrow_mut().clear();
    let mut blocks = vec![Block::new(); 10];
    for (block_index, block) in blocks.iter_mut().enumerate() {
        for (byte_index, byte) in block.contents.iter_mut().enumerate() {
            *byte = (block_index as u8).wrapping_mul(17) ^ byte_index as u8;
        }
    }
    volume_mgr
        .write_blocks(file, &blocks)
        .expect("bulk file write");
    let tail = [0xA5; 37];
    volume_mgr.write(file, &tail).expect("tail write");

    assert!(
        writes.borrow().iter().any(|(_, count)| *count > 1),
        "bulk write never reached the block device"
    );
    assert_eq!(
        volume_mgr.file_offset(file).expect("file offset"),
        (blocks.len() * Block::LEN + tail.len()) as u32
    );

    let mut expected = Vec::with_capacity(blocks.len() * Block::LEN + tail.len());
    for block in &blocks {
        expected.extend_from_slice(&block.contents);
    }
    expected.extend_from_slice(&tail);
    volume_mgr.close_file(file).expect("close file");
    let entry = volume_mgr
        .find_directory_entry(root_dir, "EMPTY.DAT")
        .expect("find persisted file");
    assert_eq!(entry.size as usize, expected.len());
    let file = volume_mgr
        .open_file_in_dir(root_dir, "EMPTY.DAT", Mode::ReadOnly)
        .expect("reopen file");
    let mut actual = vec![0; expected.len()];
    assert_eq!(
        volume_mgr.read(file, &mut actual).expect("read file"),
        actual.len()
    );
    assert_eq!(actual, expected);
}

#[test]
fn bulk_write_rejects_unaligned_offset() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let volume_mgr: VolumeManager<_, _, 4, 2, 1> =
        VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr
        .open_raw_volume(VolumeIdx(0))
        .expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");
    let file = volume_mgr
        .open_file_in_dir(root_dir, "EMPTY.DAT", Mode::ReadWriteTruncate)
        .expect("open file");

    volume_mgr.write(file, &[0x55]).expect("initial write");
    let result = volume_mgr.write_blocks(file, &[Block::new()]);
    assert!(matches!(result, Err(Error::InvalidOffset)));
    assert_eq!(volume_mgr.file_offset(file).expect("file offset"), 1);
    assert_eq!(volume_mgr.file_length(file).expect("file length"), 1);
}

#[test]
fn bulk_write_follows_fragmented_cluster_chain() {
    let time_source = utils::make_time_source();
    let (disk, writes) = make_recording_device();
    let volume_mgr: VolumeManager<_, _, 4, 2, 1> =
        VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr
        .open_raw_volume(VolumeIdx(0))
        .expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");
    let one_cluster = vec![Block::new(); 4];

    let target = volume_mgr
        .open_file_in_dir(root_dir, "EMPTY.DAT", Mode::ReadWriteTruncate)
        .expect("open target");
    volume_mgr
        .write_blocks(target, &one_cluster)
        .expect("write first target cluster");
    volume_mgr.close_file(target).expect("close target");

    let gap = volume_mgr
        .open_file_in_dir(root_dir, "GAP.DAT", Mode::ReadWriteCreate)
        .expect("create gap file");
    volume_mgr
        .write_blocks(gap, &one_cluster)
        .expect("write gap cluster");
    volume_mgr.close_file(gap).expect("close gap file");

    let target = volume_mgr
        .open_file_in_dir(root_dir, "EMPTY.DAT", Mode::ReadWriteAppend)
        .expect("reopen target");
    volume_mgr
        .write_blocks(target, &one_cluster)
        .expect("append fragmented target cluster");
    volume_mgr
        .file_seek_from_start(target, 0)
        .expect("seek target to start");

    writes.borrow_mut().clear();
    let replacement = vec![Block::new(); 8];
    volume_mgr
        .write_blocks(target, &replacement)
        .expect("overwrite fragmented file");

    let data_writes: Vec<_> = writes
        .borrow()
        .iter()
        .copied()
        .filter(|(_, count)| *count > 1)
        .collect();
    assert_eq!(data_writes.len(), 2);
    assert_eq!(data_writes[0].1, 4);
    assert_eq!(data_writes[1].1, 4);
    assert_ne!(data_writes[1].0 .0, data_writes[0].0 .0 + 4);
}
// ****************************************************************************
//
// End Of File
//
// ****************************************************************************
