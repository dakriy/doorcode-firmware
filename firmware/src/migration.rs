//! The stepping-stone image's first step on a board still on ESP-IDF's
//! standard two-OTA partition table, whose `factory`, `ota_0`, and `ota_1`
//! slots of 1 MB start at 0x10000, 0x110000, and 0x210000
//! (`docs/migration.md`). It runs before anything else touches the flash
//! or the network, and moves the board to this firmware's table
//! (`partitions.csv`), which keeps `nvs`, `otadata`, and `phy_init` where
//! they were and starts its `ota_1` at 0x210000 too:
//!
//! - Running from 0x110000 under the old table, it copies its own slot to
//!   0x210000, has the bootloader boot that slot, and restarts.
//! - Running from 0x210000 under the old table, it writes the new table
//!   over the old one at 0x8000, reads it back until it matches, and has
//!   ESP-IDF load the table again, so the next update goes into the new
//!   `ota_0` of 2 MB at 0x10000.
//!
//! Under the new table, or one it does not know, it changes nothing.

use std::ffi::c_void;
use std::ptr;
use std::thread;
use std::time::Duration;

use esp_idf_svc::sys::{self, EspError};

/// Where the bootloader reads the partition table, and its length.
const TABLE_AT: u32 = 0x8000;
const TABLE_BYTES: usize = 0xC00;
/// This firmware's partition table, as ESP-IDF's `gen_esp32part.py` makes
/// it from `partitions.csv` (see `build.rs`): its entries, the MD5 entry,
/// and padding to the table's full length.
const NEW_TABLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/partition-table.bin"));
/// The standard table's first app slot, and the length of each.
const FIRST_SLOT: u32 = 0x10000;
const SLOT_BYTES: u32 = 0x100000;
/// The standard table's `ota_0` and `ota_1`.
const OLD_OTA_0: u32 = 0x110000;
const OLD_OTA_1: u32 = 0x210000;
/// Each partition of ESP-IDF's standard two-OTA table, as type, subtype,
/// offset, and size: `nvs`, `otadata`, `phy_init`, `factory`, `ota_0`, and
/// `ota_1`.
const OLD_TABLE: [[u32; 4]; 6] = [
    [0x01, 0x02, 0x9000, 0x4000],
    [0x01, 0x00, 0xD000, 0x2000],
    [0x01, 0x01, 0xF000, 0x1000],
    [0x00, 0x00, FIRST_SLOT, SLOT_BYTES],
    [0x00, 0x10, OLD_OTA_0, SLOT_BYTES],
    [0x00, 0x11, OLD_OTA_1, SLOT_BYTES],
];
/// A table entry's length, and the two bytes each partition's starts with.
const ENTRY_BYTES: usize = 32;
const ENTRY_MAGIC: [u8; 2] = [0xAA, 0x50];
/// The flash's erase unit.
const SECTOR: u32 = 0x1000;
// The table write stays inside the one sector it erases.
const _: () = assert!(NEW_TABLE.len() == TABLE_BYTES && TABLE_BYTES <= SECTOR as usize);
/// How often the new table is written before the step gives up.
const TABLE_WRITES: usize = 3;
/// A pause after each sector copied, so the idle task runs and the task
/// watchdog stays fed through the 256 sectors of a slot.
const COPY_PAUSE: Duration = Duration::from_millis(5);

/// What the partition table at `TABLE_AT` is.
enum Table {
    New,
    Old,
    Unknown,
}

/// Moves the board to the new partition table, a step at a time; see the
/// module's documentation. A step that fails leaves the board running on
/// the table it has, and is tried again on the next start.
pub(crate) fn run() {
    if let Err(error) = step() {
        log::error!("moving to the new partition table: {error}");
    }
}

fn step() -> Result<(), EspError> {
    match table()? {
        Table::Old => from_old_table(),
        Table::New => Ok(()),
        Table::Unknown => {
            log::error!("the partition table is neither the old nor the new one; left as it is");
            Ok(())
        }
    }
}

fn table() -> Result<Table, EspError> {
    let table = read(TABLE_AT, TABLE_BYTES)?;
    if table == NEW_TABLE {
        return Ok(Table::New);
    }
    let old = partitions(&table) == OLD_TABLE;
    Ok(if old { Table::Old } else { Table::Unknown })
}

/// The type, subtype, offset, and size of each partition `table` lists,
/// up to the first entry that is not a partition (the MD5 entry or
/// padding).
fn partitions(table: &[u8]) -> Vec<[u32; 4]> {
    table
        .chunks_exact(ENTRY_BYTES)
        .take_while(|entry| entry[..2] == ENTRY_MAGIC)
        .map(|entry| {
            [
                u32::from(entry[2]),
                u32::from(entry[3]),
                word(&entry[4..8]),
                word(&entry[8..12]),
            ]
        })
        .collect()
}

fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn from_old_table() -> Result<(), EspError> {
    match running_slot() {
        OLD_OTA_0 => move_to_old_ota_1(),
        OLD_OTA_1 => write_new_table(),
        slot => {
            log::error!("running from {slot:#x} under the old partition table; left as it is");
            Ok(())
        }
    }
}

/// The start of the standard table's app slot this code runs from, read
/// from where the flash cache maps this function. The bootloader's own
/// record of it is not asked for, since ESP-IDF keeps that as a pointer
/// into the partition list, which `write_new_table` replaces.
fn running_slot() -> u32 {
    let here = run as fn() as *const c_void;
    // SAFETY: `here` is code mapped from flash; an address that is not
    // makes the call return SPI_FLASH_CACHE2PHYS_FAIL.
    let at = unsafe { sys::spi_flash_cache2phys(here) };
    let at = u32::try_from(at).unwrap_or(u32::MAX);
    FIRST_SLOT + at.saturating_sub(FIRST_SLOT) / SLOT_BYTES * SLOT_BYTES
}

/// Copies the old `ota_0`, this image, to the old `ota_1`, has the
/// bootloader boot it, and restarts. ESP-IDF's OTA writes are not used,
/// since they refuse to start while the running image is still on trial,
/// as this one is; setting the slot to boot checks the copy first.
fn move_to_old_ota_1() -> Result<(), EspError> {
    copy(OLD_OTA_0, OLD_OTA_1, SLOT_BYTES)?;
    let subtype = sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_APP_OTA_1;
    // SAFETY: the label may be null; the result is a static entry of the
    // partition table, or null, which the next call refuses.
    let slot = unsafe {
        sys::esp_partition_find_first(
            sys::esp_partition_type_t_ESP_PARTITION_TYPE_APP,
            subtype,
            ptr::null(),
        )
    };
    // SAFETY: `slot` is a static partition entry or null, which the call
    // refuses with an error.
    sys::esp!(unsafe { sys::esp_ota_set_boot_partition(slot) })?;
    esp_idf_svc::hal::reset::restart();
}

fn copy(from: u32, to: u32, length: u32) -> Result<(), EspError> {
    for offset in (0..length).step_by(SECTOR as usize) {
        let sector = read(from + offset, SECTOR as usize)?;
        erase(to + offset)?;
        write(to + offset, &sector)?;
        thread::sleep(COPY_PAUSE);
    }
    Ok(())
}

/// Writes the new table at `TABLE_AT` until it reads back the same, then
/// has ESP-IDF drop the partition list it read at start, so the next
/// lookup reads the new table.
fn write_new_table() -> Result<(), EspError> {
    for _ in 0..TABLE_WRITES {
        if table_written()? {
            // SAFETY: nothing has kept a partition pointer yet: this runs
            // before NVS, Wi-Fi, and the OTA functions are first used.
            unsafe { sys::esp_partition_unload_all() };
            return Ok(());
        }
    }
    log::error!("the new partition table did not read back as written");
    Err(EspError::from_infallible::<{ sys::ESP_FAIL }>())
}

/// Writes the new table at `TABLE_AT`, and whether it reads back the same.
fn table_written() -> Result<bool, EspError> {
    erase(TABLE_AT)?;
    write(TABLE_AT, NEW_TABLE)?;
    Ok(read(TABLE_AT, TABLE_BYTES)? == NEW_TABLE)
}

fn read(at: u32, length: usize) -> Result<Vec<u8>, EspError> {
    let mut bytes = vec![0; length];
    // SAFETY: a null chip is the main flash, and `bytes` holds `length`
    // bytes for the whole call.
    sys::esp!(unsafe {
        sys::esp_flash_read(
            ptr::null_mut(),
            bytes.as_mut_ptr().cast(),
            at,
            length as u32,
        )
    })?;
    Ok(bytes)
}

/// Erases the one sector that starts at `at`.
fn erase(at: u32) -> Result<(), EspError> {
    // SAFETY: a null chip is the main flash.
    sys::esp!(unsafe { sys::esp_flash_erase_region(ptr::null_mut(), at, SECTOR) })
}

fn write(at: u32, bytes: &[u8]) -> Result<(), EspError> {
    // SAFETY: a null chip is the main flash, and `bytes` is valid for its
    // length for the whole call.
    sys::esp!(unsafe {
        sys::esp_flash_write(
            ptr::null_mut(),
            bytes.as_ptr().cast(),
            at,
            bytes.len() as u32,
        )
    })
}
