extern crate alloc;

use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::ptr::read_unaligned;
use core::sync::atomic::{AtomicBool, Ordering};

const TAR_BLOCK_SIZE: usize = 512;
// TODO: Optimization Opportunity: replace the fixed index with heap-backed
// entries when archive size and lookup cost justify the added allocation path.
const MAX_ENTRIES: usize = 48;
const PATH_CAPACITY: usize = 120;
const DISPLAY_PATH_CAPACITY: usize = PATH_CAPACITY + 3;
const MAX_WRITABLE_FILE_SIZE: usize = 1024 * 1024;
const MAX_OVERLAY_FILES: usize = 32;
const MAX_OVERLAY_BYTES: usize = 2 * 1024 * 1024;
const MULTIBOOT_BOOTLOADER_MAGIC: u32 = 0x2bad_b002;
const MULTIBOOT_INFO_MODULES: u32 = 1 << 3;
#[cfg(test)]
const IDENTITY_MAPPED_LIMIT: usize = 16 * 1024 * 1024;
#[cfg(not(test))]
const IDENTITY_MAPPED_LIMIT: usize = crate::memory::IDENTITY_MAPPED_LIMIT;

#[derive(Clone, Copy)]
/// Indexed tar member using an archive-relative data offset.
struct FileEntry {
    path: [u8; PATH_CAPACITY],
    path_length: usize,
    data_offset: usize,
    size: usize,
    is_directory: bool,
}

impl FileEntry {
    const EMPTY: Self = Self {
        path: [0; PATH_CAPACITY],
        path_length: 0,
        data_offset: 0,
        size: 0,
        is_directory: false,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// Public directory/file metadata with a display path rooted at `C:\`.
pub struct EntryInfo {
    pub path: [u8; DISPLAY_PATH_CAPACITY],
    pub path_length: usize,
    pub size: usize,
    pub is_directory: bool,
}

/// Read-only view of the initrd bytes and its bounded path index.
pub struct Archive<'a> {
    bytes: &'a [u8],
    entries: [FileEntry; MAX_ENTRIES],
    entry_count: usize,
}

static mut ARCHIVE: Option<Archive<'static>> = None;

/// One mutable file stored in the volatile overlay above the immutable initrd.
struct WritableEntry {
    path: [u8; PATH_CAPACITY],
    path_length: usize,
    contents: Vec<u8>,
}

/// Heap-backed mutable overlay; base archive entries remain immutable.
struct WritableOverlay {
    entries: Vec<WritableEntry>,
    used_bytes: usize,
}

impl WritableOverlay {
    /// Creates an empty overlay without allocating until a write is committed.
    ///
    /// NOTE: Open-source Contribution Area: reserve a bounded initial index
    /// only after measuring boot-time heap availability.
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
            used_bytes: 0,
        }
    }
}

/// Serializes overlay reads and writes while preserving allocator access.
struct OverlayLock {
    locked: AtomicBool,
    overlay: UnsafeCell<WritableOverlay>,
}

// SAFETY: all access to `overlay` is protected by the atomic lock and its guard.
unsafe impl Sync for OverlayLock {}

impl OverlayLock {
    /// Constructs a statically initialized overlay lock.
    ///
    /// NOTE: Open-source Contribution Area: use the kernel's shared
    /// synchronization primitive if one is introduced for filesystem state.
    const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
            overlay: UnsafeCell::new(WritableOverlay::new()),
        }
    }

    /// Disables local interrupts and serializes access to the overlay.
    ///
    /// NOTE: Open-source Contribution Area: add lock-rank checking if the
    /// kernel grows additional nested filesystem and storage locks.
    fn lock(&self) -> OverlayGuard<'_> {
        let interrupt_state = disable_interrupts();
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        OverlayGuard {
            lock: self,
            interrupt_state,
        }
    }
}

/// RAII guard for the writable overlay's protected state.
struct OverlayGuard<'a> {
    lock: &'a OverlayLock,
    interrupt_state: usize,
}

impl Deref for OverlayGuard<'_> {
    type Target = WritableOverlay;

    /// Borrows the overlay while the associated spin lock is held.
    ///
    /// NOTE: Open-source Contribution Area: keep guard borrowing exclusive to
    /// the lifetime of the acquired lock.
    fn deref(&self) -> &Self::Target {
        // SAFETY: this guard owns the lock until Drop.
        unsafe { &*self.lock.overlay.get() }
    }
}

impl DerefMut for OverlayGuard<'_> {
    /// Borrows the overlay mutably while the associated spin lock is held.
    ///
    /// NOTE: Open-source Contribution Area: audit this unsafe access if lock
    /// ownership or scheduler preemption semantics change.
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: this guard exclusively owns the lock until Drop.
        unsafe { &mut *self.lock.overlay.get() }
    }
}

impl Drop for OverlayGuard<'_> {
    /// Releases the lock before restoring the prior interrupt state.
    ///
    /// NOTE: Open-source Contribution Area: preserve release ordering and
    /// interrupt restoration when adapting this guard to other architectures.
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        restore_interrupts(self.interrupt_state);
    }
}

static WRITABLE_OVERLAY: OverlayLock = OverlayLock::new();

#[repr(C)]
/// Multiboot fields needed to find the loaded initrd module.
struct MultibootInfoPrefix {
    flags: u32,
    _memory_lower: u32,
    _memory_upper: u32,
    _boot_device: u32,
    _command_line: u32,
    module_count: u32,
    module_address: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
/// Physical address range and command-line tag for a boot module.
struct MultibootModule {
    start: u32,
    end: u32,
    command_line: u32,
    _reserved: u32,
}

/// Locates the `initrd` Multiboot module and indexes its tar archive.
///
/// `boot_magic` and `multiboot_info_address` are supplied by the bootloader.
/// Returns an error if module metadata is absent, invalid, or outside the
/// current identity map.
///
/// # Safety
/// The information address and module descriptors must refer to valid,
/// readable Multiboot data in the mapped low-memory range.
pub fn initialize_from_multiboot(
    boot_magic: u32,
    multiboot_info_address: u32,
) -> Result<(), &'static str> {
    if boot_magic != MULTIBOOT_BOOTLOADER_MAGIC {
        return Err("invalid Multiboot boot magic while locating initrd");
    }
    let info = unsafe { read_unaligned(multiboot_info_address as *const MultibootInfoPrefix) };
    if info.flags & MULTIBOOT_INFO_MODULES == 0 || info.module_count == 0 {
        return Err("GRUB did not provide any boot modules");
    }
    let module_table_size = (info.module_count as usize)
        .checked_mul(core::mem::size_of::<MultibootModule>())
        .ok_or("Multiboot module table size overflow")?;
    let module_table_end = (info.module_address as usize)
        .checked_add(module_table_size)
        .ok_or("Multiboot module table address overflow")?;
    if module_table_end > IDENTITY_MAPPED_LIMIT {
        return Err("Multiboot module table is outside the kernel identity map");
    }

    let mut selected = None;
    for index in 0..info.module_count as usize {
        let module_address =
            info.module_address as usize + index * core::mem::size_of::<MultibootModule>();
        let module = unsafe { read_unaligned(module_address as *const MultibootModule) };
        if module.end <= module.start {
            return Err("Multiboot module has an invalid memory range");
        }
        if command_line_equals(module.command_line, b"initrd")? {
            selected = Some(module);
            break;
        }
    }
    let module = selected.ok_or("GRUB did not provide the initrd module")?;
    let start = module.start as usize;
    let length = (module.end - module.start) as usize;
    let end = start
        .checked_add(length)
        .ok_or("initrd memory range overflow")?;
    if end > IDENTITY_MAPPED_LIMIT {
        return Err("initrd is outside the kernel identity map");
    }
    let bytes = unsafe { core::slice::from_raw_parts(start as *const u8, length) };
    initialize(bytes)
}

/// Parses and installs a static tar archive as the read-only `C:\` drive.
///
/// `bytes` must remain valid for the lifetime of the kernel. Returns an error
/// for malformed tar entries, unsupported member kinds, or index exhaustion.
pub fn initialize(bytes: &'static [u8]) -> Result<(), &'static str> {
    let archive = Archive::parse(bytes)?;
    unsafe {
        ARCHIVE = Some(archive);
    }
    Ok(())
}

/// Returns the number of indexed paths, including the synthetic drive root.
///
/// NOTE: Open-source Contribution Area: add indexed directory snapshots if
/// File Explorer needs stable pagination during concurrent writes.
pub fn entry_count() -> Result<usize, &'static str> {
    let overlay = WRITABLE_OVERLAY.lock();
    with_archive(|archive| {
        let additional = overlay
            .entries
            .iter()
            .filter(|entry| {
                !archive.entries[..archive.entry_count].iter().any(|base| {
                    base.path_length == entry.path_length
                        && base.path[..base.path_length]
                            .eq_ignore_ascii_case(&entry.path[..entry.path_length])
                })
            })
            .count();
        archive.entry_count + additional + 1
    })
}

/// Returns directory entry `index`, or `None` when it is beyond the index.
///
/// Index zero is the synthetic `C:\` root. Errors are returned if the archive
/// has not been initialized.
///
/// NOTE: Open-source Contribution Area: expose a generation-tagged snapshot
/// when directory pagination must remain stable during writes.
pub fn entry_at(index: usize) -> Result<Option<EntryInfo>, &'static str> {
    let overlay = WRITABLE_OVERLAY.lock();
    with_archive(|archive| {
        if index == 0 {
            return Some(root_entry());
        }
        let mut visible_index = 1usize;
        for base in &archive.entries[..archive.entry_count] {
            let replacement = overlay.entries.iter().find(|entry| {
                entry.path_length == base.path_length
                    && entry.path[..entry.path_length]
                        .eq_ignore_ascii_case(&base.path[..base.path_length])
            });
            let info = replacement
                .map(display_writable_entry)
                .unwrap_or_else(|| display_entry(base));
            if visible_index == index {
                return Some(info);
            }
            visible_index += 1;
        }
        for entry in &overlay.entries {
            let exists_in_base = archive.entries[..archive.entry_count].iter().any(|base| {
                base.path_length == entry.path_length
                    && base.path[..base.path_length]
                        .eq_ignore_ascii_case(&entry.path[..entry.path_length])
            });
            if exists_in_base {
                continue;
            }
            if visible_index == index {
                return Some(display_writable_entry(entry));
            }
            visible_index += 1;
        }
        None
    })
}

/// Resolves a drive-rooted Windows path using case-insensitive matching.
///
/// Accepts either slash style below `C:\`. Returns an error for invalid paths,
/// an uninitialized archive, or a path that does not exist.
///
/// NOTE: Open-source Contribution Area: add mount-aware canonicalization and
/// explicit case policy as further filesystems are introduced.
pub fn resolve_windows_path(path: &str) -> Result<EntryInfo, &'static str> {
    let normalized = normalize_windows_path(path)?;
    let overlay = WRITABLE_OVERLAY.lock();
    if let Some(entry) = overlay.entries.iter().find(|entry| {
        entry.path_length == normalized.length
            && entry.path[..entry.path_length]
                .eq_ignore_ascii_case(&normalized.bytes[..normalized.length])
    }) {
        return Ok(display_writable_entry(entry));
    }
    with_archive(|archive| {
        if normalized.length == 0 {
            return Some(root_entry());
        }
        archive.entries[..archive.entry_count]
            .iter()
            .find(|entry| {
                entry.path_length == normalized.length
                    && entry.path[..entry.path_length]
                        .eq_ignore_ascii_case(&normalized.bytes[..normalized.length])
            })
            .map(display_entry)
    })?
    .ok_or("path was not found on C:\\")
}

/// Copies a file's bytes into `destination` and returns the copied length.
///
/// `path` must name a file on `C:\`; directories are rejected. Returns an
/// error if the path is invalid or missing, the destination is too small, or
/// the archive is not initialized.
///
/// NOTE: Open-source Contribution Area: add consistency guarantees for
/// multi-block files once a persistent backend exists.
pub fn read_file(path: &str, destination: &mut [u8]) -> Result<usize, &'static str> {
    let normalized = normalize_windows_path(path)?;
    {
        let overlay = WRITABLE_OVERLAY.lock();
        if let Some(entry) = overlay.entries.iter().find(|entry| {
            entry.path_length == normalized.length
                && entry.path[..entry.path_length]
                    .eq_ignore_ascii_case(&normalized.bytes[..normalized.length])
        }) {
            if destination.len() < entry.contents.len() {
                return Err("destination buffer is too small for the file");
            }
            destination[..entry.contents.len()].copy_from_slice(&entry.contents);
            return Ok(entry.contents.len());
        }
    }
    with_archive(|archive| {
        let entry = archive.entries[..archive.entry_count]
            .iter()
            .find(|entry| {
                !entry.is_directory
                    && entry.path_length == normalized.length
                    && entry.path[..entry.path_length]
                        .eq_ignore_ascii_case(&normalized.bytes[..normalized.length])
            })
            .ok_or("file was not found on C:\\")?;
        if destination.len() < entry.size {
            return Err("destination buffer is too small for the file");
        }
        let end = entry
            .data_offset
            .checked_add(entry.size)
            .ok_or("file data range overflow")?;
        destination[..entry.size].copy_from_slice(&archive.bytes[entry.data_offset..end]);
        Ok(entry.size)
    })?
}

/// Atomically creates or replaces a regular file in the volatile C: overlay.
///
/// The read-only initrd remains untouched: writes are copied into heap-owned
/// overlay storage and become visible to subsequent VFS reads/listings. The
/// parent directory must already exist, directories cannot be replaced, and
/// overlay file/count/aggregate-byte limits are checked before committing.
///
/// # Errors
/// Returns an error for invalid/traversal paths, missing parent directories,
/// allocation or quota exhaustion, oversized files, and write attempts to a
/// directory. On failure, existing file contents and overlay metadata remain
/// unchanged.
///
/// NOTE: Open-source Contribution Area: replace the volatile overlay with a
/// journaled storage backend only after block-device flush and recovery
/// semantics are implemented and tested.
pub fn write_file(path: &str, data: &[u8]) -> Result<(), &'static str> {
    let normalized = normalize_windows_path(path)?;
    if normalized.length == 0 {
        return Err("cannot write the C:\\ drive root");
    }
    if data.len() > MAX_WRITABLE_FILE_SIZE {
        return Err("file exceeds the writable overlay file-size limit");
    }
    validate_writable_parent(&normalized)?;
    if let Some(existing) = archive_entry(&normalized)? {
        if existing.is_directory {
            return Err("cannot replace a directory with a file");
        }
    }

    // Allocate and copy the payload before taking the overlay lock. A failed
    // allocation therefore cannot partially replace a previously saved file.
    let mut contents = Vec::new();
    contents
        .try_reserve_exact(data.len())
        .map_err(|_| "not enough heap memory for file contents")?;
    contents.extend_from_slice(data);

    let mut overlay = WRITABLE_OVERLAY.lock();
    let existing_index = overlay.entries.iter().position(|entry| {
        entry.path_length == normalized.length
            && entry.path[..entry.path_length]
                .eq_ignore_ascii_case(&normalized.bytes[..normalized.length])
    });
    let previous_size = existing_index
        .map(|index| overlay.entries[index].contents.len())
        .unwrap_or(0);
    let next_total = overlay
        .used_bytes
        .checked_sub(previous_size)
        .and_then(|used| used.checked_add(contents.len()))
        .ok_or("writable overlay byte count overflow")?;
    if next_total > MAX_OVERLAY_BYTES {
        return Err("writable overlay exceeds its total byte limit");
    }

    if let Some(index) = existing_index {
        overlay.entries[index].contents = contents;
        overlay.used_bytes = next_total;
        return Ok(());
    }
    if overlay.entries.len() >= MAX_OVERLAY_FILES {
        return Err("writable overlay contains too many files");
    }
    overlay
        .entries
        .try_reserve(1)
        .map_err(|_| "not enough heap memory for the writable file index")?;
    let mut stored_path = [0u8; PATH_CAPACITY];
    stored_path[..normalized.length].copy_from_slice(&normalized.bytes[..normalized.length]);
    overlay.entries.push(WritableEntry {
        path: stored_path,
        path_length: normalized.length,
        contents,
    });
    overlay.used_bytes = next_total;
    Ok(())
}

impl<'a> Archive<'a> {
    /// Parses a ustar-like byte archive and builds its bounded path index.
    ///
    /// `bytes` is retained by reference for later file reads. Returns an error
    /// on malformed headers, unsupported entry types, duplicate/conflicting
    /// paths, or more than `MAX_ENTRIES` indexed paths.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, &'static str> {
        let mut archive = Self {
            bytes,
            entries: [FileEntry::EMPTY; MAX_ENTRIES],
            entry_count: 0,
        };
        let mut offset = 0usize;
        let mut found_end = false;

        while offset < bytes.len() {
            let header_end = offset
                .checked_add(TAR_BLOCK_SIZE)
                .ok_or("tar header offset overflow")?;
            if header_end > bytes.len() {
                return Err("truncated tar header");
            }
            // Tar headers occupy one 512-byte block. Validate the checksum and
            // file bounds before advancing over the padded payload.
            let header = &bytes[offset..header_end];
            if header.iter().all(|byte| *byte == 0) {
                found_end = true;
                break;
            }
            validate_checksum(header)?;

            let name = field_bytes(header, 0, 100)?;
            let prefix = field_bytes(header, 345, 155)?;
            let kind = header[156];
            if kind != 0 && kind != b'0' && kind != b'5' {
                return Err("initrd contains an unsupported tar entry type");
            }
            let size = if kind == b'5' {
                0
            } else {
                parse_octal(&header[124..136])?
            };
            let data_offset = header_end;
            let data_end = data_offset
                .checked_add(size)
                .ok_or("tar file data offset overflow")?;
            if data_end > bytes.len() {
                return Err("tar file data extends past the initrd");
            }
            let padded_size = size
                .checked_add(TAR_BLOCK_SIZE - 1)
                .ok_or("tar padded size overflow")?
                / TAR_BLOCK_SIZE
                * TAR_BLOCK_SIZE;
            offset = data_offset
                .checked_add(padded_size)
                .ok_or("tar next-header offset overflow")?;
            if offset > bytes.len() {
                return Err("tar padding extends past the initrd");
            }

            let mut path = [0u8; PATH_CAPACITY];
            let path_length = join_tar_path(&mut path, prefix, name)?;
            let path_length = trim_trailing_slashes(&path, path_length);
            validate_path(&path[..path_length])?;
            archive.add_with_parents(&path[..path_length], data_offset, size, kind == b'5')?;
        }
        if !found_end {
            return Err("tar archive is missing its end marker");
        }
        Ok(archive)
    }

    fn add_with_parents(
        &mut self,
        path: &[u8],
        data_offset: usize,
        size: usize,
        is_directory: bool,
    ) -> Result<(), &'static str> {
        for separator in path
            .iter()
            .enumerate()
            .filter_map(|(index, byte)| (*byte == b'/').then_some(index))
        {
            self.add_entry(&path[..separator], 0, 0, true)?;
        }
        self.add_entry(path, data_offset, size, is_directory)
    }

    fn add_entry(
        &mut self,
        path: &[u8],
        data_offset: usize,
        size: usize,
        is_directory: bool,
    ) -> Result<(), &'static str> {
        if let Some(existing) = self.entries[..self.entry_count].iter_mut().find(|entry| {
            entry.path_length == path.len() && entry.path[..entry.path_length] == *path
        }) {
            if existing.is_directory && !is_directory {
                return Err("initrd path is both a directory and a file");
            }
            if !existing.is_directory && !is_directory {
                return Err("initrd contains a duplicate file path");
            }
            if !is_directory {
                *existing = make_entry(path, data_offset, size, false)?;
            }
            return Ok(());
        }
        if self.entry_count == MAX_ENTRIES {
            return Err("initrd contains too many indexed paths");
        }
        // NOTE: Open-source Contribution Area: replace this small fixed index
        // with a fallible heap-backed structure if larger initrds are needed.
        self.entries[self.entry_count] = make_entry(path, data_offset, size, is_directory)?;
        self.entry_count += 1;
        Ok(())
    }
}

fn with_archive<R>(operation: impl FnOnce(&Archive<'static>) -> R) -> Result<R, &'static str> {
    // Initialization happens once before task scheduling; the archive bytes are
    // boot-module memory retained for the kernel lifetime and never mutated.
    unsafe {
        (&*core::ptr::addr_of!(ARCHIVE))
            .as_ref()
            .map(operation)
            .ok_or("C:\\ virtual drive is not initialized")
    }
}

/// Finds a normalized path in the immutable boot archive.
///
/// The returned metadata borrows no archive memory, so callers may safely
/// release any overlay lock before copying file payload bytes.
///
/// NOTE: Open-source Contribution Area: centralize VFS lookup across archive
/// and future mounted backends as the storage model grows.
fn archive_entry(path: &VecPath) -> Result<Option<EntryInfo>, &'static str> {
    with_archive(|archive| {
        archive.entries[..archive.entry_count]
            .iter()
            .find(|entry| {
                entry.path_length == path.length
                    && entry.path[..entry.path_length]
                        .eq_ignore_ascii_case(&path.bytes[..path.length])
            })
            .map(display_entry)
    })
}

/// Rejects writes unless the normalized path's parent is an indexed directory.
///
/// The current overlay creates files only; it deliberately does not synthesize
/// arbitrary directories or permit path traversal into a future mount.
///
/// NOTE: Open-source Contribution Area: add directory creation and mount-aware
/// parent resolution only with corresponding authorization and rollback rules.
fn validate_writable_parent(path: &VecPath) -> Result<(), &'static str> {
    let parent_length = path.bytes[..path.length]
        .iter()
        .rposition(|byte| *byte == b'/')
        .unwrap_or(0);
    if parent_length == 0 {
        return Ok(());
    }
    let parent = VecPath {
        bytes: path.bytes,
        length: parent_length,
    };
    match archive_entry(&parent)? {
        Some(entry) if entry.is_directory => Ok(()),
        Some(_) => Err("parent path is not a directory"),
        None => Err("parent directory does not exist"),
    }
}

/// Creates display metadata for one overlay entry without exposing its payload.
///
/// NOTE: Open-source Contribution Area: share path-display conversion between
/// archive, overlay, and future persistent storage backends.
fn display_writable_entry(entry: &WritableEntry) -> EntryInfo {
    let mut info = root_entry();
    info.path[3..3 + entry.path_length].copy_from_slice(&entry.path[..entry.path_length]);
    for byte in &mut info.path[3..3 + entry.path_length] {
        if *byte == b'/' {
            *byte = b'\\';
        }
    }
    info.path_length = 3 + entry.path_length;
    info.size = entry.contents.len();
    info.is_directory = false;
    info
}

/// Captures x86 EFLAGS and disables local interrupts before locking the overlay.
///
/// NOTE: Open-source Contribution Area: preserve architecture-specific
/// interrupt state when porting the kernel beyond x86.
#[cfg(all(not(test), target_arch = "x86"))]
fn disable_interrupts() -> usize {
    let flags: usize;
    unsafe {
        core::arch::asm!(
            "pushfd",
            "pop {flags:e}",
            "cli",
            flags = out(reg) flags,
            options(nomem, nostack)
        );
    }
    flags
}

/// Supplies the test/non-x86 interrupt-state placeholder for overlay locking.
///
/// NOTE: Open-source Contribution Area: replace this placeholder if tests or
/// production builds are moved to another interrupt architecture.
#[cfg(any(test, not(target_arch = "x86")))]
fn disable_interrupts() -> usize {
    0
}

/// Restores the caller's x86 interrupt-enable state after releasing the lock.
///
/// NOTE: Open-source Contribution Area: pair this with the architecture's
/// preemption-control rules if the scheduler gains SMP support.
#[cfg(all(not(test), target_arch = "x86"))]
fn restore_interrupts(flags: usize) {
    if flags & (1 << 9) != 0 {
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack));
        }
    }
}

/// Supplies the test/non-x86 restore placeholder for overlay locking.
///
/// NOTE: Open-source Contribution Area: implement the matching architecture
/// restore path alongside `disable_interrupts` when needed.
#[cfg(any(test, not(target_arch = "x86")))]
fn restore_interrupts(_flags: usize) {}

fn root_entry() -> EntryInfo {
    let mut root = EntryInfo {
        path: [0; DISPLAY_PATH_CAPACITY],
        path_length: 3,
        size: 0,
        is_directory: true,
    };
    root.path[..3].copy_from_slice(b"C:\\");
    root
}

fn make_entry(
    path: &[u8],
    data_offset: usize,
    size: usize,
    is_directory: bool,
) -> Result<FileEntry, &'static str> {
    if path.len() > PATH_CAPACITY {
        return Err("initrd path exceeds index capacity");
    }
    let mut entry = FileEntry::EMPTY;
    entry.path[..path.len()].copy_from_slice(path);
    entry.path_length = path.len();
    entry.data_offset = data_offset;
    entry.size = size;
    entry.is_directory = is_directory;
    Ok(entry)
}

fn display_entry(entry: &FileEntry) -> EntryInfo {
    let mut info = root_entry();
    let path_length = entry.path_length.min(PATH_CAPACITY);
    info.path[3..3 + path_length].copy_from_slice(&entry.path[..path_length]);
    for byte in &mut info.path[3..3 + path_length] {
        if *byte == b'/' {
            *byte = b'\\';
        }
    }
    info.path_length = 3 + path_length;
    info.size = entry.size;
    info.is_directory = entry.is_directory;
    info
}

fn normalize_windows_path(path: &str) -> Result<VecPath, &'static str> {
    let bytes = path.as_bytes();
    let remainder = if bytes.len() >= 3
        && bytes[0].eq_ignore_ascii_case(&b'C')
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        &bytes[3..]
    } else if bytes.eq_ignore_ascii_case(b"C:") {
        &[]
    } else {
        return Err("path must be rooted at C:\\");
    };
    let mut normalized = VecPath {
        bytes: [0; PATH_CAPACITY],
        length: 0,
    };
    for component in remainder
        .split(|byte| *byte == b'\\' || *byte == b'/')
        .filter(|component| !component.is_empty())
    {
        if component == b"." || component == b".." || !component.is_ascii() {
            return Err("path contains an invalid component");
        }
        if normalized.length != 0 {
            push_path_byte(&mut normalized, b'/')?;
        }
        for byte in component {
            push_path_byte(&mut normalized, byte.to_ascii_lowercase())?;
        }
    }
    Ok(normalized)
}

/// Stack-local normalized path representation used by drive lookup.
struct VecPath {
    bytes: [u8; PATH_CAPACITY],
    length: usize,
}

fn push_path_byte(path: &mut VecPath, byte: u8) -> Result<(), &'static str> {
    if path.length == path.bytes.len() {
        return Err("path exceeds VFS lookup capacity");
    }
    path.bytes[path.length] = byte;
    path.length += 1;
    Ok(())
}

fn command_line_equals(address: u32, expected: &[u8]) -> Result<bool, &'static str> {
    // Module command lines are raw bootloader pointers. Bound the expected
    // tag and NUL terminator before reading bytes from the identity map.
    let Some(command_line_end) = (address as usize).checked_add(expected.len() + 1) else {
        return Ok(false);
    };
    if address == 0 || command_line_end > IDENTITY_MAPPED_LIMIT {
        return Ok(false);
    }
    for (index, expected_byte) in expected.iter().enumerate() {
        // SAFETY: the address range was checked against the current identity map.
        let byte = unsafe { core::ptr::read((address as *const u8).add(index)) };
        if byte != *expected_byte {
            return Ok(false);
        }
    }
    // SAFETY: the checked span includes the expected trailing NUL byte.
    Ok(unsafe { core::ptr::read((address as *const u8).add(expected.len())) == 0 })
}

fn field_bytes(header: &[u8], offset: usize, capacity: usize) -> Result<&[u8], &'static str> {
    let end = offset
        .checked_add(capacity)
        .ok_or("tar header field range overflow")?;
    let field = header
        .get(offset..end)
        .ok_or("tar header field is truncated")?;
    let length = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    Ok(&field[..length])
}

fn join_tar_path(
    destination: &mut [u8; PATH_CAPACITY],
    prefix: &[u8],
    name: &[u8],
) -> Result<usize, &'static str> {
    let mut length = 0;
    for part in [prefix, name] {
        if part.is_empty() {
            continue;
        }
        if length != 0 {
            if length == destination.len() {
                return Err("initrd path exceeds index capacity");
            }
            destination[length] = b'/';
            length += 1;
        }
        let end = length
            .checked_add(part.len())
            .ok_or("initrd path length overflow")?;
        if end > destination.len() {
            return Err("initrd path exceeds index capacity");
        }
        destination[length..end].copy_from_slice(part);
        length = end;
    }
    Ok(length)
}

fn trim_trailing_slashes(path: &[u8], mut length: usize) -> usize {
    while length > 0 && path[length - 1] == b'/' {
        length -= 1;
    }
    length
}

fn validate_path(path: &[u8]) -> Result<(), &'static str> {
    if path.is_empty() || !path.is_ascii() {
        return Err("initrd contains an empty or non-ASCII path");
    }
    for component in path.split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." || component == b".." {
            return Err("initrd path contains an unsafe component");
        }
    }
    Ok(())
}

fn parse_octal(field: &[u8]) -> Result<usize, &'static str> {
    let mut value = 0usize;
    let mut saw_digit = false;
    for byte in field
        .iter()
        .copied()
        .skip_while(|byte| *byte == 0 || *byte == b' ')
    {
        if byte == 0 || byte == b' ' {
            break;
        }
        if !(b'0'..=b'7').contains(&byte) {
            return Err("tar numeric field is not valid octal");
        }
        saw_digit = true;
        value = value
            .checked_mul(8)
            .and_then(|current| current.checked_add((byte - b'0') as usize))
            .ok_or("tar numeric field overflow")?;
    }
    if saw_digit {
        Ok(value)
    } else {
        Ok(0)
    }
}

fn validate_checksum(header: &[u8]) -> Result<(), &'static str> {
    let stored = parse_octal(
        header
            .get(148..156)
            .ok_or("tar checksum field is truncated")?,
    )?;
    let sum: usize = header
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                b' ' as usize
            } else {
                *byte as usize
            }
        })
        .sum();
    if sum == stored {
        Ok(())
    } else {
        Err("tar header checksum does not match")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        entry_at, entry_count, initialize, read_file, resolve_windows_path, write_file, Archive,
        MAX_ENTRIES, TAR_BLOCK_SIZE,
    };
    use alloc::boxed::Box;
    use alloc::vec::Vec;

    /// Builds one checksum-correct ustar record for filesystem regression tests.
    ///
    /// NOTE: Open-source Contribution Area: add reusable archive fixtures for
    /// nested and malformed paths as more VFS behavior is covered.
    fn header(name: &[u8], data: &[u8], kind: u8) -> Vec<u8> {
        let mut block = [0u8; TAR_BLOCK_SIZE];
        block[..name.len()].copy_from_slice(name);
        block[100..108].copy_from_slice(b"0000644\0");
        block[108..116].copy_from_slice(b"0000000\0");
        block[116..124].copy_from_slice(b"0000000\0");
        let size = if kind == b'5' { 0 } else { data.len() };
        let size_text = format!("{size:011o}\0");
        block[124..136].copy_from_slice(size_text.as_bytes());
        block[136..148].copy_from_slice(b"00000000000\0");
        block[148..156].fill(b' ');
        block[156] = kind;
        block[257..263].copy_from_slice(b"ustar\0");
        block[263..265].copy_from_slice(b"00");
        let checksum: usize = block.iter().map(|byte| *byte as usize).sum();
        let checksum_text = format!("{checksum:06o}\0 ");
        block[148..156].copy_from_slice(checksum_text.as_bytes());

        let mut archive = block.to_vec();
        if kind != b'5' {
            archive.extend_from_slice(data);
            archive.resize(
                (archive.len() + TAR_BLOCK_SIZE - 1) / TAR_BLOCK_SIZE * TAR_BLOCK_SIZE,
                0,
            );
        }
        archive
    }

    /// Verifies overlay create, replace, read, listing, and failure atomicity.
    ///
    /// NOTE: Open-source Contribution Area: add quota-boundary and concurrent
    /// writer tests as the overlay policy and scheduler evolve.
    #[test]
    fn writes_overlay_files_without_mutating_archive_entries() {
        let mut bytes = header(b"Docs/", b"", b'5');
        bytes.extend_from_slice(&header(b"Docs/BASE.TXT", b"base bytes", b'0'));
        bytes.extend_from_slice(&[0; TAR_BLOCK_SIZE * 2]);
        let archive: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        initialize(archive).unwrap();

        assert_eq!(
            resolve_windows_path(r"C:\Docs\BASE.TXT").unwrap().size,
            b"base bytes".len()
        );
        write_file(r"C:\Docs\notes.txt", b"first draft").unwrap();
        let created = resolve_windows_path(r"c:/docs/NOTES.TXT").unwrap();
        assert_eq!(created.size, b"first draft".len());
        assert!(!created.is_directory);

        let mut output = [0u8; 32];
        let length = read_file(r"C:\Docs\notes.txt", &mut output).unwrap();
        assert_eq!(&output[..length], b"first draft");
        let count_after_create = entry_count().unwrap();
        assert_eq!(count_after_create, 4);
        assert!((0..count_after_create)
            .filter_map(|index| entry_at(index).unwrap())
            .any(|entry| &entry.path[..entry.path_length] == b"C:\\docs\\notes.txt"));

        write_file(r"C:\Docs\BASE.TXT", b"replacement").unwrap();
        let length = read_file(r"C:\Docs\BASE.TXT", &mut output).unwrap();
        assert_eq!(&output[..length], b"replacement");
        assert_eq!(entry_count().unwrap(), count_after_create);

        assert!(write_file(r"C:\Missing\bad.txt", b"bad").is_err());
        let length = read_file(r"C:\Docs\BASE.TXT", &mut output).unwrap();
        assert_eq!(&output[..length], b"replacement");
    }

    #[test]
    fn indexes_directories_files_and_windows_paths() {
        let mut bytes = header(b"Docs/", b"", b'5');
        bytes.extend_from_slice(&header(b"Docs/NOTES.TXT", b"hello", b'0'));
        bytes.extend_from_slice(&[0; TAR_BLOCK_SIZE * 2]);
        let archive = Archive::parse(&bytes).unwrap();
        assert!(archive.entry_count < MAX_ENTRIES);
        assert!(archive.entries[..archive.entry_count]
            .iter()
            .any(|entry| entry.path[..entry.path_length] == *b"Docs" && entry.is_directory));
        assert!(archive.entries[..archive.entry_count]
            .iter()
            .any(|entry| entry.path[..entry.path_length] == *b"Docs/NOTES.TXT" && entry.size == 5));
        let note = archive.entries[..archive.entry_count]
            .iter()
            .find(|entry| entry.path[..entry.path_length] == *b"Docs/NOTES.TXT")
            .unwrap();
        assert_eq!(
            &archive.bytes[note.data_offset..note.data_offset + note.size],
            b"hello"
        );
        let normalized = super::normalize_windows_path("c:\\docs\\notes.txt").unwrap();
        assert_eq!(&normalized.bytes[..normalized.length], b"docs/notes.txt");
    }

    #[test]
    fn rejects_bad_headers_and_traversal_paths() {
        let mut malformed = header(b"../escape.txt", b"bad", b'0');
        malformed.extend_from_slice(&[0; TAR_BLOCK_SIZE * 2]);
        assert!(Archive::parse(&malformed).is_err());

        let mut invalid_checksum = header(b"safe.txt", b"ok", b'0');
        invalid_checksum[0] ^= 1;
        invalid_checksum.extend_from_slice(&[0; TAR_BLOCK_SIZE * 2]);
        assert!(Archive::parse(&invalid_checksum).is_err());
    }
}
