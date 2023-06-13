use core::ffi::c_void;
use std::convert::TryFrom;
use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::mem;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsFd;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::BorrowedFd;
use std::os::unix::io::FromRawFd;
use std::os::unix::io::OwnedFd;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::ptr;
use std::ptr::null;
use std::ptr::NonNull;
use std::slice::from_raw_parts;

use bitflags::bitflags;
use libbpf_sys::bpf_map_info;
use libbpf_sys::bpf_obj_get_info_by_fd;
use nix::errno;
use nix::unistd;
use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;
use strum_macros::Display;

use crate::util;
use crate::util::parse_ret_i32;
use crate::Error;
use crate::Link;
use crate::Result;

/// Represents a parsed but not yet loaded BPF map.
///
/// This object exposes operations that need to happen before the map is created.
///
/// Some methods require working with raw bytes. You may find libraries such as
/// [`plain`](https://crates.io/crates/plain) helpful.
#[derive(Debug)]
pub struct OpenMap {
    ptr: NonNull<libbpf_sys::bpf_map>,
}

// TODO: Document members.
#[allow(missing_docs)]
impl OpenMap {
    /// Create a new [`OpenMap`] from a ptr to a `libbpf_sys::bpf_map`.
    ///
    /// # Safety
    /// The pointer must point to an opened but not loaded map.
    pub(crate) unsafe fn new(ptr: NonNull<libbpf_sys::bpf_map>) -> Self {
        Self { ptr }
    }

    /// Retrieve the `Map`'s name.
    pub fn name(&self) -> Result<&str> {
        let name_ptr = unsafe { libbpf_sys::bpf_map__name(self.ptr.as_ptr()) };
        let name_c_str = unsafe { CStr::from_ptr(name_ptr) };
        name_c_str
            .to_str()
            .map_err(|e| Error::Internal(e.to_string()))
    }

    /// Retrieve type of the map.
    pub fn map_type(&self) -> MapType {
        let ty = unsafe { libbpf_sys::bpf_map__type(self.ptr.as_ptr()) };
        match MapType::try_from(ty) {
            Ok(t) => t,
            Err(_) => MapType::Unknown,
        }
    }

    pub fn set_map_ifindex(&mut self, idx: u32) {
        unsafe { libbpf_sys::bpf_map__set_ifindex(self.ptr.as_ptr(), idx) };
    }

    pub fn set_initial_value(&mut self, data: &[u8]) -> Result<()> {
        let ret = unsafe {
            libbpf_sys::bpf_map__set_initial_value(
                self.ptr.as_ptr(),
                data.as_ptr() as *const std::ffi::c_void,
                data.len() as libbpf_sys::size_t,
            )
        };

        util::parse_ret(ret)
    }

    pub fn set_type(&mut self, ty: MapType) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_map__set_type(self.ptr.as_ptr(), ty as u32) };
        util::parse_ret(ret)
    }

    pub fn set_key_size(&mut self, size: u32) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_map__set_key_size(self.ptr.as_ptr(), size) };
        util::parse_ret(ret)
    }

    pub fn set_value_size(&mut self, size: u32) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_map__set_value_size(self.ptr.as_ptr(), size) };
        util::parse_ret(ret)
    }

    pub fn set_max_entries(&mut self, count: u32) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_map__set_max_entries(self.ptr.as_ptr(), count) };
        util::parse_ret(ret)
    }

    pub fn set_map_flags(&mut self, flags: u32) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_map__set_map_flags(self.ptr.as_ptr(), flags) };
        util::parse_ret(ret)
    }

    pub fn set_numa_node(&mut self, numa_node: u32) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_map__set_numa_node(self.ptr.as_ptr(), numa_node) };
        util::parse_ret(ret)
    }

    pub fn set_inner_map_fd(&mut self, inner: &Map) {
        unsafe {
            libbpf_sys::bpf_map__set_inner_map_fd(self.ptr.as_ptr(), inner.as_fd().as_raw_fd())
        };
    }

    pub fn set_map_extra(&mut self, map_extra: u64) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_map__set_map_extra(self.ptr.as_ptr(), map_extra) };
        util::parse_ret(ret)
    }

    pub fn set_autocreate(&mut self, autocreate: bool) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_map__set_autocreate(self.ptr.as_ptr(), autocreate) };
        util::parse_ret(ret)
    }

    pub fn set_pin_path<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path_c = util::path_to_cstring(path)?;
        let path_ptr = path_c.as_ptr();

        let ret = unsafe { libbpf_sys::bpf_map__set_pin_path(self.ptr.as_ptr(), path_ptr) };
        util::parse_ret(ret)
    }

    /// Reuse an fd for a BPF map
    pub fn reuse_fd(&self, fd: i32) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_map__reuse_fd(self.ptr.as_ptr(), fd) };
        util::parse_ret(ret)
    }

    /// Reuse an already-pinned map for `self`.
    pub fn reuse_pinned_map<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let cstring = util::path_to_cstring(path)?;

        let fd = unsafe { libbpf_sys::bpf_obj_get(cstring.as_ptr()) };
        if fd < 0 {
            return Err(Error::System(errno::errno()));
        }
        let reuse_result = self.reuse_fd(fd);

        // Always close `fd` regardless of if `reuse_fd` succeeded or failed
        //
        // Ignore errors b/c can't really recover from failure
        let _ = unistd::close(fd);

        reuse_result
    }
}

#[derive(Debug)]
enum MapFd {
    Owned(OwnedFd),
    Borrowed(RawFd),
}

impl AsFd for MapFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        match self {
            Self::Owned(o) => o.as_fd(),
            Self::Borrowed(fd) => unsafe {
                // SAFETY
                // This filedescriptor is open because of two invariants:
                // - This variant is only constructed in `Map::new`, which is the entry point for
                // when the map doesn't own the descriptor
                // - That method is crate private and called only by the `Object` which has its own
                // invariant that it outlives every `Map` it owns and cleans then up when dropped,
                // thus this fd must be live.
                BorrowedFd::borrow_raw(*fd)
            },
        }
    }
}

impl AsRawFd for MapFd {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::Owned(o) => o.as_raw_fd(),
            Self::Borrowed(fd) => *fd,
        }
    }
}

/// Represents a created map.
///
/// Some methods require working with raw bytes. You may find libraries such as
/// [`plain`](https://crates.io/crates/plain) helpful.
#[derive(Debug)]
pub struct Map {
    handle: MapHandle,
    // The ptr will be null if we use Map::create to create the map from the userspace side directly.
    ptr: Option<NonNull<libbpf_sys::bpf_map>>,
}

impl Map {
    /// Create a [`Map`] from a [`libbpf_sys::bpf_map`].
    ///
    /// # Safety
    ///
    /// The pointer must point to a loaded map.
    pub(crate) unsafe fn new(ptr: NonNull<libbpf_sys::bpf_map>) -> Result<Self> {
        // Get the map name
        // SAFETY:
        // bpf_map__name can return null but only if it's passed a null.
        // We already know ptr is not null.
        let name = unsafe { libbpf_sys::bpf_map__name(ptr.as_ptr()) };
        let name = util::c_ptr_to_string(name)?;

        // Get the map fd
        let fd = unsafe { libbpf_sys::bpf_map__fd(ptr.as_ptr()) };
        if fd < 0 {
            return Err(Error::System(-fd));
        }

        let ty = MapType::try_from(unsafe { libbpf_sys::bpf_map__type(ptr.as_ptr()) })
            .unwrap_or(MapType::Unknown);
        let key_size = unsafe { libbpf_sys::bpf_map__key_size(ptr.as_ptr()) };
        let value_size = unsafe { libbpf_sys::bpf_map__value_size(ptr.as_ptr()) };

        let handle = MapHandle {
            fd: MapFd::Borrowed(fd),
            name,
            ty,
            key_size,
            value_size,
        };

        Ok(Map {
            handle,
            ptr: Some(ptr),
        })
    }

    /// Open a previously pinned map from its path.
    ///
    /// # Panics
    /// If the path contains null bytes.
    pub fn from_pinned_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        fn inner(path: &Path) -> Result<Map> {
            let p = CString::new(path.as_os_str().as_bytes()).expect("path contained null bytes");
            let fd = parse_ret_i32(unsafe {
                // SAFETY
                // p is never null since we allocated ourselves.
                libbpf_sys::bpf_obj_get(p.as_ptr())
            })?;
            Map::from_fd(unsafe {
                // SAFETY
                // A file descriptor coming from the bpf_obj_get function is always suitable for
                // ownership and can be cleaned up with close.
                OwnedFd::from_raw_fd(fd)
            })
        }

        inner(path.as_ref())
    }

    /// Open a loaded map from its map id.
    pub fn from_map_id(id: u32) -> Result<Self> {
        parse_ret_i32(unsafe {
            // SAFETY
            // This function is always safe to call.
            libbpf_sys::bpf_map_get_fd_by_id(id)
        })
        .map(|fd| unsafe {
            // SAFETY
            // A file descriptor coming from the bpf_map_get_fd_by_id function is always suitable
            // for ownership and can be cleaned up with close.
            OwnedFd::from_raw_fd(fd)
        })
        .and_then(Self::from_fd)
    }

    fn from_fd(fd: OwnedFd) -> Result<Self> {
        let info = MapInfo::new(fd.as_fd())?;
        let handle = MapHandle::new(
            fd,
            info.name()?.into(),
            info.map_type(),
            info.info.key_size,
            info.info.value_size,
        );
        Ok(Self { handle, ptr: None })
    }

    /// Fetch extra map information
    pub fn info(&self) -> Result<MapInfo> {
        MapInfo::new(self.as_fd())
    }

    /// Returns a file descriptor to the underlying map.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.as_fd()
    }

    /// [Pin](https://facebookmicrosites.github.io/bpf/blog/2018/08/31/object-lifetime.html#bpffs)
    /// this map to bpffs.
    pub fn pin<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path_c = util::path_to_cstring(path)?;
        let path_ptr = path_c.as_ptr();

        let ret = match self.ptr {
            Some(ptr) => unsafe { libbpf_sys::bpf_map__pin(ptr.as_ptr(), path_ptr) },
            None => unsafe { libbpf_sys::bpf_obj_pin(self.as_fd().as_raw_fd(), path_ptr) },
        };

        util::parse_ret(ret)
    }

    /// [Unpin](https://facebookmicrosites.github.io/bpf/blog/2018/08/31/object-lifetime.html#bpffs)
    /// from bpffs
    pub fn unpin<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        match self.ptr {
            Some(ptr) => {
                let path_c = util::path_to_cstring(path)?;
                let path_ptr = path_c.as_ptr();
                let ret = unsafe { libbpf_sys::bpf_map__unpin(ptr.as_ptr(), path_ptr) };
                util::parse_ret(ret)
            }
            None => match std::fs::remove_file(path) {
                Ok(_) => Ok(()),
                Err(e) => Err(Error::Internal(format!("remove pin map failed: {e}"))),
            },
        }
    }

    /// Returns whether map is pinned or not flag
    pub fn is_pinned(&self) -> Result<bool> {
        match self.ptr {
            Some(ptr) => Ok(unsafe { libbpf_sys::bpf_map__is_pinned(ptr.as_ptr()) }),
            None => Err(Error::InvalidInput(("No map pointer found").to_string())),
        }
    }

    /// Returns the pin_path if the map is pinned, otherwise, None is returned
    pub fn get_pin_path(&self) -> Option<&OsStr> {
        let path_ptr = match self.ptr {
            Some(ptr) => unsafe { libbpf_sys::bpf_map__pin_path(ptr.as_ptr()) },
            None => return None,
        };
        if path_ptr.is_null() {
            // means map is not pinned
            return None;
        }
        let path_c_str = unsafe { CStr::from_ptr(path_ptr) };
        Some(OsStr::from_bytes(path_c_str.to_bytes()))
    }

    /// Freeze the map as read-only from user space.
    ///
    /// Entries from a frozen map can no longer be updated or deleted with the
    /// bpf() system call. This operation is not reversible, and the map remains
    /// immutable from user space until its destruction. However, read and write
    /// permissions for BPF programs to the map remain unchanged.
    pub fn freeze(&self) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_map_freeze(self.as_fd().as_raw_fd()) };

        util::parse_ret(ret)
    }

    /// Returns an iterator over keys in this map
    ///
    /// Note that if the map is not stable (stable meaning no updates or deletes) during iteration,
    /// iteration can skip keys, restart from the beginning, or duplicate keys. In other words,
    /// iteration becomes unpredictable.
    pub fn keys(&self) -> MapKeyIter {
        MapKeyIter::new(self, self.key_size())
    }

    /// Create the bpf map standalone.
    pub fn create<T: AsRef<str>>(
        map_type: MapType,
        name: Option<T>,
        key_size: u32,
        value_size: u32,
        max_entries: u32,
        opts: &libbpf_sys::bpf_map_create_opts,
    ) -> Result<Map> {
        let (map_name_str, map_name) = match name {
            Some(name) => (
                util::str_to_cstring(name.as_ref())?,
                name.as_ref().to_string(),
            ),

            // The old version kernel don't support specifying map name, we can use 'Option::<&str>::None' for the name argument.
            None => (util::str_to_cstring("")?, "".to_string()),
        };

        let map_name_ptr = {
            if map_name_str.as_bytes().is_empty() {
                null()
            } else {
                map_name_str.as_ptr()
            }
        };

        let fd = unsafe {
            libbpf_sys::bpf_map_create(
                map_type.into(),
                map_name_ptr,
                key_size,
                value_size,
                max_entries,
                opts,
            )
        };
        let () = util::parse_ret(fd)?;

        let handle = MapHandle::new(
            unsafe {
                // SAFETY
                // A file descriptor coming from the bpf_map_create function is always suitable for
                // ownership and can be cleaned up with close.
                OwnedFd::from_raw_fd(fd)
            },
            map_name,
            map_type,
            key_size,
            value_size,
        );

        Ok(Map { handle, ptr: None })
    }

    /// Attach a struct ops map
    pub fn attach_struct_ops(&self) -> Result<Link> {
        if self.map_type() != MapType::StructOps {
            return Err(Error::InvalidInput(format!(
                "Invalid map type ({}) for attach_struct_ops()",
                self.map_type(),
            )));
        }

        let ptr = match self.ptr {
            Some(ptr) => ptr,
            None => {
                return Err(Error::InvalidInput(
                    "Cannot attach a user-created struct_ops map".to_string(),
                ))
            }
        };

        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_map__attach_struct_ops(ptr.as_ptr())
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Retrieve the underlying [`libbpf_sys::bpf_map`].
    pub fn as_libbpf_bpf_map_ptr(&self) -> Option<NonNull<libbpf_sys::bpf_map>> {
        self.ptr
    }

    /// Create a new [`MapHandle`] from this map.
    pub fn to_handle(&self) -> Result<MapHandle> {
        self.handle.try_clone()
    }
}

impl AsFd for Map {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.handle.as_fd()
    }
}

impl BpfMap for Map {
    fn name(&self) -> &str {
        self.handle.name()
    }

    fn map_type(&self) -> MapType {
        self.handle.map_type()
    }

    fn key_size(&self) -> u32 {
        self.handle.key_size()
    }

    fn value_size(&self) -> u32 {
        self.handle.value_size()
    }

    fn lookup(&self, key: &[u8], flags: MapFlags) -> Result<Option<Vec<u8>>> {
        self.handle.lookup(key, flags)
    }

    fn lookup_percpu(&self, key: &[u8], flags: MapFlags) -> Result<Option<Vec<Vec<u8>>>> {
        self.handle.lookup_percpu(key, flags)
    }

    fn delete(&self, key: &[u8]) -> Result<()> {
        self.handle.delete(key)
    }

    fn delete_batch(&self, keys: &[u8], count: u32, elem_flags: MapFlags, flags: MapFlags) -> Result<()> {
        self.handle.delete_batch(keys, count, elem_flags, flags)
    }

    fn lookup_and_delete(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.handle.lookup_and_delete(key)
    }

    fn update(&self, key: &[u8], value: &[u8], flags: MapFlags) -> Result<()> {
        self.handle.update(key, value, flags)
    }

    fn update_percpu(&self, key: &[u8], values: &[Vec<u8>], flags: MapFlags) -> Result<()> {
        self.handle.update_percpu(key, values, flags)
    }
}

impl From<Map> for OwnedFd {
    fn from(map: Map) -> Self {
        match map.handle.fd {
            MapFd::Owned(o) => o,
            MapFd::Borrowed(_) => unreachable!(
                "it shouldn't be possible to have an owned map that doesn't own its fd"
            ),
        }
    }
}

impl TryFrom<OwnedFd> for Map {
    type Error = Error;

    fn try_from(fd: OwnedFd) -> Result<Self> {
        Map::from_fd(fd)
    }
}

/// A lightweight, restricted handle to a map.
///
/// Some methods require working with raw bytes. You may find libraries such as
/// [`plain`](https://crates.io/crates/plain) helpful.
#[derive(Debug)]
pub struct MapHandle {
    fd: MapFd,
    name: String,
    ty: MapType,
    key_size: u32,
    value_size: u32,
}

impl MapHandle {
    /// Create a [`MapHandle`] from raw data.
    fn new(fd: OwnedFd, name: String, ty: MapType, key_size: u32, value_size: u32) -> Self {
        let fd = MapFd::Owned(fd);
        MapHandle {
            fd,
            name,
            ty,
            key_size,
            value_size,
        }
    }

    /// Try cloning this handle by duplicating its underlying file descriptor.
    pub fn try_clone(&self) -> Result<Self> {
        let new_fd = self
            .as_fd()
            .try_clone_to_owned()
            .map_err(|e| Error::Internal(e.to_string()))?;
        let fd = MapFd::Owned(new_fd);
        Ok(MapHandle {
            fd,
            name: self.name.clone(),
            ty: self.ty,
            key_size: self.key_size,
            value_size: self.value_size,
        })
    }

    /// Return the size of one value including padding for interacting with per-cpu
    /// maps. The values are aligned to 8 bytes.
    fn percpu_aligned_value_size(&self) -> usize {
        util::roundup(self.value_size() as usize, 8)
    }

    /// Returns the size of the buffer needed for a lookup/update of a per-cpu map.
    fn percpu_buffer_size(&self) -> Result<usize> {
        let aligned_val_size = self.percpu_aligned_value_size();
        let ncpu = crate::num_possible_cpus()?;
        Ok(ncpu * aligned_val_size)
    }

    /// Internal function to return a value from a map into a buffer of the given size.
    fn lookup_raw(
        &self,
        key: &[u8],
        flags: MapFlags,
        out_size: usize,
    ) -> Result<Option<Vec<u8>>> {
        if key.len() != self.key_size() as usize {
            return Err(Error::InvalidInput(format!(
                "key_size {} != {}",
                key.len(),
                self.key_size()
            )));
        };

        let mut out: Vec<u8> = Vec::with_capacity(out_size);

        let ret = unsafe {
            libbpf_sys::bpf_map_lookup_elem_flags(
                self.as_fd().as_raw_fd(),
                key.as_ptr() as *const c_void,
                out.as_mut_ptr() as *mut c_void,
                flags.bits,
            )
        };

        if ret == 0 {
            unsafe {
                out.set_len(out_size);
            }
            Ok(Some(out))
        } else {
            let errno = errno::errno();
            if errno::Errno::from_i32(errno) == errno::Errno::ENOENT {
                Ok(None)
            } else {
                Err(Error::System(errno))
            }
        }
    }

    /// Internal function to update a map. This does not check the length of the
    /// supplied value.
    fn update_raw(&self, key: &[u8], value: &[u8], flags: MapFlags) -> Result<()> {
        if key.len() != self.key_size() as usize {
            return Err(Error::InvalidInput(format!(
                "key_size {} != {}",
                key.len(),
                self.key_size()
            )));
        };

        let ret = unsafe {
            libbpf_sys::bpf_map_update_elem(
                self.as_fd().as_raw_fd(),
                key.as_ptr() as *const c_void,
                value.as_ptr() as *const c_void,
                flags.bits,
            )
        };

        util::parse_ret(ret)
    }
}

impl AsFd for MapHandle {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// Represents a map or handle to a map
pub trait BpfMap: AsFd {
    /// Retrieve the `Map`'s name.
    fn name(&self) -> &str;

    /// Retrieve type of the map.
    fn map_type(&self) -> MapType;

    /// Key size in bytes
    fn key_size(&self) -> u32;

    /// Value size in bytes
    fn value_size(&self) -> u32;

    /// Returns map value as `Vec` of `u8`.
    ///
    /// `key` must have exactly [`Map::key_size()`] elements.
    ///
    /// If the map is one of the per-cpu data structures, the function [`Map::lookup_percpu()`]
    /// must be used.
    fn lookup(&self, key: &[u8], flags: MapFlags) -> Result<Option<Vec<u8>>>;

    /// Returns one value per cpu as `Vec` of `Vec` of `u8` for per per-cpu maps.
    ///
    /// For normal maps, [`Map::lookup()`] must be used.
    fn lookup_percpu(&self, key: &[u8], flags: MapFlags) -> Result<Option<Vec<Vec<u8>>>>;

    /// Deletes an element from the map.
    ///
    /// `key` must have exactly [`Map::key_size()`] elements.
    fn delete(&self, key: &[u8]) -> Result<()>;

    /// Deletes many elements in batch mode from the map.
    ///
    /// `keys` must have exactly [`Map::key_size()` * count] elements.
    fn delete_batch(
        &self,
        keys: &[u8],
        count: u32,
        elem_flags: MapFlags,
        flags: MapFlags,
    ) -> Result<()>;

    /// Same as [`Map::lookup()`] except this also deletes the key from the map.
    ///
    /// Note that this operation is currently only implemented in the kernel for [`MapType::Queue`]
    /// and [`MapType::Stack`].
    ///
    /// `key` must have exactly [`Map::key_size()`] elements.
    fn lookup_and_delete(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    /// Update an element.
    ///
    /// `key` must have exactly [`Map::key_size()`] elements. `value` must have exactly
    /// [`Map::value_size()`] elements.
    ///
    /// For per-cpu maps, [`Map::update_percpu()`] must be used.
    fn update(&self, key: &[u8], value: &[u8], flags: MapFlags) -> Result<()>;

    /// Update an element in an per-cpu map with one value per cpu.
    ///
    /// `key` must have exactly [`Map::key_size()`] elements. `value` must have one
    /// element per cpu (see [`num_possible_cpus`][crate::num_possible_cpus])
    /// with exactly [`Map::value_size()`] elements each.
    ///
    /// For per-cpu maps, [`Map::update_percpu()`] must be used.
    fn update_percpu(&self, key: &[u8], values: &[Vec<u8>], flags: MapFlags) -> Result<()>;
}

impl BpfMap for MapHandle {
    fn name(&self) -> &str {
        &self.name
    }

    fn map_type(&self) -> MapType {
        self.ty
    }

    fn key_size(&self) -> u32 {
        self.key_size
    }

    fn value_size(&self) -> u32 {
        self.value_size
    }

    fn lookup(&self, key: &[u8], flags: MapFlags) -> Result<Option<Vec<u8>>> {
        if self.map_type().is_percpu() {
            return Err(Error::InvalidInput(format!(
                "lookup_percpu() must be used for per-cpu maps (type of the map is {})",
                self.map_type(),
            )));
        }

        let out_size = self.value_size() as usize;
        self.lookup_raw(key, flags, out_size)
    }

    fn lookup_percpu(&self, key: &[u8], flags: MapFlags) -> Result<Option<Vec<Vec<u8>>>> {
        if !self.map_type().is_percpu() && self.map_type() != MapType::Unknown {
            return Err(Error::InvalidInput(format!(
                "lookup() must be used for maps that are not per-cpu (type of the map is {})",
                self.map_type(),
            )));
        }

        let val_size = self.value_size() as usize;
        let aligned_val_size = self.percpu_aligned_value_size();
        let out_size = self.percpu_buffer_size()?;

        let raw_res = self.lookup_raw(key, flags, out_size)?;
        if let Some(raw_vals) = raw_res {
            let mut out = Vec::new();
            for chunk in raw_vals.chunks_exact(aligned_val_size) {
                out.push(chunk[..val_size].to_vec());
            }
            Ok(Some(out))
        } else {
            Ok(None)
        }
    }

    fn delete(&self, key: &[u8]) -> Result<()> {
        if key.len() != self.key_size() as usize {
            return Err(Error::InvalidInput(format!(
                "key_size {} != {}",
                key.len(),
                self.key_size()
            )));
        };

        let ret = unsafe {
            libbpf_sys::bpf_map_delete_elem(self.as_fd().as_raw_fd(), key.as_ptr() as *const c_void)
        };
        util::parse_ret(ret)
    }

    fn delete_batch(
        &self,
        keys: &[u8],
        count: u32,
        elem_flags: MapFlags,
        flags: MapFlags,
    ) -> Result<()> {
        if keys.len() as u32 / count != self.key_size() || (keys.len() as u32) % count != 0 {
            return Err(Error::InvalidInput(format!(
                "batch key_size {} != {} * {}",
                keys.len(),
                self.key_size(),
                count
            )));
        };

        let opts = libbpf_sys::bpf_map_batch_opts {
            sz: mem::size_of::<libbpf_sys::bpf_map_batch_opts>() as u64,
            elem_flags: elem_flags.bits,
            flags: flags.bits,
        };

        let mut count = count;
        let ret = unsafe {
            libbpf_sys::bpf_map_delete_batch(
                self.as_fd().as_raw_fd(),
                keys.as_ptr() as *const c_void,
                (&mut count) as *mut u32,
                &opts as *const libbpf_sys::bpf_map_batch_opts,
            )
        };
        util::parse_ret(ret)
    }

    fn lookup_and_delete(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if key.len() != self.key_size() as usize {
            return Err(Error::InvalidInput(format!(
                "key_size {} != {}",
                key.len(),
                self.key_size()
            )));
        };

        let mut out: Vec<u8> = Vec::with_capacity(self.value_size() as usize);

        let ret = unsafe {
            libbpf_sys::bpf_map_lookup_and_delete_elem(
                self.as_fd().as_raw_fd(),
                key.as_ptr() as *const c_void,
                out.as_mut_ptr() as *mut c_void,
            )
        };

        if ret == 0 {
            unsafe {
                out.set_len(self.value_size() as usize);
            }
            Ok(Some(out))
        } else {
            let errno = errno::errno();
            if errno::Errno::from_i32(errno) == errno::Errno::ENOENT {
                Ok(None)
            } else {
                Err(Error::System(errno))
            }
        }
    }

    fn update(&self, key: &[u8], value: &[u8], flags: MapFlags) -> Result<()> {
        if self.map_type().is_percpu() {
            return Err(Error::InvalidInput(format!(
                "update_percpu() must be used for per-cpu maps (type of the map is {})",
                self.map_type(),
            )));
        }

        if value.len() != self.value_size() as usize {
            return Err(Error::InvalidInput(format!(
                "value_size {} != {}",
                value.len(),
                self.value_size()
            )));
        };

        self.update_raw(key, value, flags)
    }

    fn update_percpu(&self, key: &[u8], values: &[Vec<u8>], flags: MapFlags) -> Result<()> {
        if !self.map_type().is_percpu() && self.map_type() != MapType::Unknown {
            return Err(Error::InvalidInput(format!(
                "update() must be used for maps that are not per-cpu (type of the map is {})",
                self.map_type(),
            )));
        }

        if values.len() != crate::num_possible_cpus()? {
            return Err(Error::InvalidInput(format!(
                "number of values {} != number of cpus {}",
                values.len(),
                crate::num_possible_cpus()?
            )));
        };

        let val_size = self.value_size() as usize;
        let aligned_val_size = self.percpu_aligned_value_size();
        let buf_size = self.percpu_buffer_size()?;

        let mut value_buf = Vec::new();
        value_buf.resize(buf_size, 0);

        for (i, val) in values.iter().enumerate() {
            if val.len() != val_size {
                return Err(Error::InvalidInput(format!(
                    "value size for cpu {} is {} != {}",
                    i,
                    val.len(),
                    val_size
                )));
            }

            value_buf[(i * aligned_val_size)..(i * aligned_val_size + val_size)]
                .copy_from_slice(val);
        }

        self.update_raw(key, &value_buf, flags)
    }
}

bitflags! {
    /// Flags to configure [`Map`] operations.
    pub struct MapFlags: u64 {
        /// See [`libbpf_sys::BPF_ANY`].
        const ANY      = libbpf_sys::BPF_ANY as _;
        /// See [`libbpf_sys::BPF_NOEXIST`].
        const NO_EXIST = libbpf_sys::BPF_NOEXIST as _;
        /// See [`libbpf_sys::BPF_EXIST`].
        const EXIST    = libbpf_sys::BPF_EXIST as _;
        /// See [`libbpf_sys::BPF_F_LOCK`].
        const LOCK     = libbpf_sys::BPF_F_LOCK as _;
    }
}

/// Type of a [`Map`]. Maps to `enum bpf_map_type` in kernel uapi.
// If you add a new per-cpu map, also update `is_percpu`.
#[non_exhaustive]
#[repr(u32)]
#[derive(Copy, Clone, TryFromPrimitive, IntoPrimitive, PartialEq, Eq, Display, Debug)]
// TODO: Document members.
#[allow(missing_docs)]
pub enum MapType {
    Unspec = 0,
    Hash,
    Array,
    ProgArray,
    PerfEventArray,
    PercpuHash,
    PercpuArray,
    StackTrace,
    CgroupArray,
    LruHash,
    LruPercpuHash,
    LpmTrie,
    ArrayOfMaps,
    HashOfMaps,
    Devmap,
    Sockmap,
    Cpumap,
    Xskmap,
    Sockhash,
    CgroupStorage,
    ReuseportSockarray,
    PercpuCgroupStorage,
    Queue,
    Stack,
    SkStorage,
    DevmapHash,
    StructOps,
    RingBuf,
    InodeStorage,
    TaskStorage,
    BloomFilter,
    /// We choose to specify our own "unknown" type here b/c it's really up to the kernel
    /// to decide if it wants to reject the map. If it accepts it, it just means whoever
    /// using this library is a bit out of date.
    Unknown = u32::MAX,
}

impl MapType {
    /// Returns if the map is of one of the per-cpu types.
    pub fn is_percpu(&self) -> bool {
        matches!(
            self,
            MapType::PercpuArray
                | MapType::PercpuHash
                | MapType::LruPercpuHash
                | MapType::PercpuCgroupStorage
        )
    }

    /// Detects if host kernel supports this BPF map type.
    ///
    /// Make sure the process has required set of CAP_* permissions (or runs as
    /// root) when performing feature checking.
    pub fn is_supported(&self) -> Result<bool> {
        let ret = unsafe { libbpf_sys::libbpf_probe_bpf_map_type(*self as u32, std::ptr::null()) };
        match ret {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::System(-ret)),
        }
    }
}

#[derive(Debug)]
pub struct MapKeyIter<'a> {
    map: &'a Map,
    prev: Option<Vec<u8>>,
    next: Vec<u8>,
}

impl<'a> MapKeyIter<'a> {
    fn new(map: &'a Map, key_size: u32) -> Self {
        Self {
            map,
            prev: None,
            next: vec![0; key_size as usize],
        }
    }
}

impl<'a> Iterator for MapKeyIter<'a> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        let prev = self.prev.as_ref().map_or(ptr::null(), |p| p.as_ptr());

        let ret = unsafe {
            libbpf_sys::bpf_map_get_next_key(
                self.map.as_fd().as_raw_fd(),
                prev as _,
                self.next.as_mut_ptr() as _,
            )
        };
        if ret != 0 {
            None
        } else {
            self.prev = Some(self.next.clone());
            Some(self.next.clone())
        }
    }
}

/// A convenience wrapper for [`bpf_map_info`][libbpf_sys::bpf_map_info]. It
/// provides the ability to retrieve the details of a certain map.
#[derive(Debug)]
pub struct MapInfo {
    /// The inner [`bpf_map_info`][libbpf_sys::bpf_map_info] object.
    pub info: bpf_map_info,
}

impl MapInfo {
    /// Create a `MapInfo` object from a fd.
    pub fn new(fd: BorrowedFd) -> Result<Self> {
        // SAFETY: `bpf_map_info` is valid for any bit pattern.
        let mut map_info = unsafe { mem::zeroed::<bpf_map_info>() };
        let mut size = mem::size_of_val(&map_info) as u32;
        // SAFETY: All pointers are derived from references and hence valid.
        let () = util::parse_ret(unsafe {
            bpf_obj_get_info_by_fd(
                fd.as_raw_fd(),
                &mut map_info as *mut bpf_map_info as *mut c_void,
                &mut size as *mut u32,
            )
        })?;
        Ok(Self { info: map_info })
    }

    /// Get the map type
    #[inline]
    pub fn map_type(&self) -> MapType {
        match MapType::try_from(self.info.type_) {
            Ok(t) => t,
            Err(_) => MapType::Unknown,
        }
    }

    /// Get the name of this map.
    ///
    /// Returns error if the underlying data in the structure is not a valid
    /// utf-8 string.
    pub fn name<'a>(&self) -> Result<&'a str> {
        // SAFETY: convert &[i8] to &[u8], and then cast that to &str. i8 and u8 has the same size.
        let char_slice = unsafe {
            from_raw_parts(
                self.info.name[..].as_ptr() as *const u8,
                self.info.name.len(),
            )
        };
        let mut zero_idx = 0;
        while zero_idx < char_slice.len() && char_slice[zero_idx] != 0 {
            zero_idx += 1;
        }
        if zero_idx == char_slice.len() {
            return Err(Error::Internal(
                "No nul found in `bpf_map_info::name`".to_string(),
            ));
        }
        CStr::from_bytes_with_nul(&char_slice[..=zero_idx])
            .map_err(|e| Error::Internal(format!("Failed to cast name to CStr: {e}")))?
            .to_str()
            .map_err(|e| Error::Internal(format!("Failed to cast name to str: {e}")))
    }

    /// Get the map flags.
    #[inline]
    pub fn flags(&self) -> MapFlags {
        MapFlags::from_bits_truncate(self.info.map_flags as u64)
    }
}
