//! `IStream` views over one exclusive, anchored Windows file handle.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use windows::core::implement;
use windows::Win32::Foundation::{E_FAIL, STG_E_INVALIDFUNCTION, STG_E_MEDIUMFULL, S_FALSE};
use windows::Win32::System::Com::{
    IAgileObject, IAgileObject_Impl, ISequentialStream_Impl, IStream, IStream_Impl, STATFLAG,
    STATSTG, STGC, STGM_READWRITE, STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
};

/// Every clone shares the same no-replace leaf handle but has its own logical
/// seek pointer. The common lock serializes repositioning and I/O, so one
/// view's cursor never leaks into another view through the Win32 file pointer.
pub(super) fn handle_backed_stream(mut file: File) -> std::io::Result<IStream> {
    let cursor = file.stream_position()?;
    Ok(HandleBackedStream::new(Arc::new(Mutex::new(file)), cursor).into())
}

#[implement(IStream, IAgileObject)]
struct HandleBackedStream {
    file: Arc<Mutex<File>>,
    cursor: Mutex<u64>,
}

impl HandleBackedStream {
    fn new(file: Arc<Mutex<File>>, cursor: u64) -> Self {
        Self {
            file,
            cursor: Mutex::new(cursor),
        }
    }

    fn io<T>(result: std::io::Result<T>) -> std::result::Result<T, windows::core::HRESULT> {
        result.map_err(|_| E_FAIL)
    }

    fn locked_file(&self) -> std::sync::MutexGuard<'_, File> {
        self.file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn locked_cursor(&self) -> std::sync::MutexGuard<'_, u64> {
        self.cursor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Lock order is always common file, then this view's cursor. This keeps a
    /// seek+operation+cursor update atomic while allowing clones to remain
    /// agile without sharing a mutable seek pointer.
    fn read_at_cursor(
        &self,
        output: &mut [u8],
    ) -> std::result::Result<usize, windows::core::HRESULT> {
        let mut file = self.locked_file();
        let mut cursor = self.locked_cursor();
        Self::io(file.seek(SeekFrom::Start(*cursor)))?;
        let read = Self::io(file.read(output))?;
        *cursor = (*cursor)
            .checked_add(read as u64)
            .ok_or(STG_E_INVALIDFUNCTION)?;
        Ok(read)
    }

    fn write_at_cursor(&self, input: &[u8]) -> std::result::Result<usize, windows::core::HRESULT> {
        let mut file = self.locked_file();
        let mut cursor = self.locked_cursor();
        (*cursor)
            .checked_add(input.len() as u64)
            .ok_or(STG_E_INVALIDFUNCTION)?;
        Self::io(file.seek(SeekFrom::Start(*cursor)))?;
        let written = Self::io(file.write(input))?;
        *cursor = (*cursor)
            .checked_add(written as u64)
            .ok_or(STG_E_INVALIDFUNCTION)?;
        Ok(written)
    }

    fn advance(cursor: u64, by: i64) -> std::result::Result<u64, windows::core::HRESULT> {
        if by >= 0 {
            cursor.checked_add(by as u64).ok_or(STG_E_INVALIDFUNCTION)
        } else {
            cursor
                .checked_sub(by.unsigned_abs())
                .ok_or(STG_E_INVALIDFUNCTION)
        }
    }
}

impl ISequentialStream_Impl for HandleBackedStream_Impl {
    fn Read(
        &self,
        pv: *mut core::ffi::c_void,
        cb: u32,
        pcbread: *mut u32,
    ) -> windows::core::HRESULT {
        if cb == 0 {
            if !pcbread.is_null() {
                unsafe { pcbread.write(0) };
            }
            return windows::core::HRESULT(0);
        }
        if pv.is_null() {
            return E_FAIL;
        }
        match self.read_at_cursor(unsafe { std::slice::from_raw_parts_mut(pv.cast(), cb as usize) })
        {
            Ok(read) => {
                if !pcbread.is_null() {
                    unsafe { pcbread.write(read as u32) };
                }
                if read == cb as usize {
                    windows::core::HRESULT(0)
                } else {
                    S_FALSE
                }
            }
            Err(error) => error,
        }
    }

    fn Write(
        &self,
        pv: *const core::ffi::c_void,
        cb: u32,
        pcbwritten: *mut u32,
    ) -> windows::core::HRESULT {
        if cb == 0 {
            if !pcbwritten.is_null() {
                unsafe { pcbwritten.write(0) };
            }
            return windows::core::HRESULT(0);
        }
        if pv.is_null() {
            return E_FAIL;
        }
        match self.write_at_cursor(unsafe { std::slice::from_raw_parts(pv.cast(), cb as usize) }) {
            Ok(written) => {
                if !pcbwritten.is_null() {
                    unsafe { pcbwritten.write(written as u32) };
                }
                if written == cb as usize {
                    windows::core::HRESULT(0)
                } else {
                    S_FALSE
                }
            }
            Err(error) => error,
        }
    }
}

impl IStream_Impl for HandleBackedStream_Impl {
    fn Seek(&self, move_by: i64, origin: STREAM_SEEK, next: *mut u64) -> windows::core::Result<()> {
        // This private writer refuses a negative absolute displacement rather
        // than reinterpreting it as a far-past-end unsigned offset. Return
        // before taking or changing either view state or `next`.
        if origin == STREAM_SEEK_SET && move_by < 0 {
            return Err(STG_E_INVALIDFUNCTION.into());
        }
        let file = self.locked_file();
        let mut cursor = self.locked_cursor();
        let position = if origin == STREAM_SEEK_SET {
            move_by as u64
        } else if origin == STREAM_SEEK_CUR {
            HandleBackedStream::advance(*cursor, move_by)?
        } else if origin == STREAM_SEEK_END {
            HandleBackedStream::advance(HandleBackedStream::io(file.metadata())?.len(), move_by)?
        } else {
            return Err(STG_E_INVALIDFUNCTION.into());
        };
        *cursor = position;
        if !next.is_null() {
            unsafe { next.write(position) };
        }
        Ok(())
    }

    fn SetSize(&self, size: u64) -> windows::core::Result<()> {
        let file = self.locked_file();
        let _cursor = self.locked_cursor();
        // `IStream::SetSize` does not alter this view's seek pointer, including
        // when a smaller size leaves it past the new end of stream.
        HandleBackedStream::io(file.set_len(size)).map_err(windows::core::Error::from)
    }

    fn CopyTo(
        &self,
        destination: windows::core::Ref<IStream>,
        bytes: u64,
        read_total: *mut u64,
        written_total: *mut u64,
    ) -> windows::core::Result<()> {
        let destination = destination.ok()?;
        let mut remaining = bytes;
        let mut read = 0_u64;
        let mut written = 0_u64;
        let mut buffer = [0_u8; 32 * 1024];
        while remaining != 0 {
            let request = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| windows::core::Error::from(STG_E_INVALIDFUNCTION))?;
            let copied = self
                .read_at_cursor(&mut buffer[..request])
                .map_err(windows::core::Error::from)?;
            if copied == 0 {
                break;
            }
            let mut offset = 0_usize;
            while offset < copied {
                let request = u32::try_from(copied - offset)
                    .map_err(|_| windows::core::Error::from(STG_E_INVALIDFUNCTION))?;
                let mut advanced = 0_u32;
                let status = unsafe {
                    destination.Write(
                        buffer[offset..copied].as_ptr().cast(),
                        request,
                        Some(&mut advanced),
                    )
                };
                if status.is_err() || advanced == 0 || advanced as usize > copied - offset {
                    return Err(if status.is_err() {
                        status
                    } else {
                        STG_E_MEDIUMFULL
                    }
                    .into());
                }
                offset = offset
                    .checked_add(advanced as usize)
                    .ok_or_else(|| windows::core::Error::from(STG_E_INVALIDFUNCTION))?;
            }
            read = read
                .checked_add(copied as u64)
                .ok_or_else(|| windows::core::Error::from(STG_E_INVALIDFUNCTION))?;
            written = written
                .checked_add(copied as u64)
                .ok_or_else(|| windows::core::Error::from(STG_E_INVALIDFUNCTION))?;
            remaining -= copied as u64;
            if copied < request {
                break;
            }
        }
        if !read_total.is_null() {
            unsafe { read_total.write(read) };
        }
        if !written_total.is_null() {
            unsafe { written_total.write(written) };
        }
        Ok(())
    }

    fn Commit(&self, _flags: &STGC) -> windows::core::Result<()> {
        HandleBackedStream::io(self.locked_file().sync_data()).map_err(windows::core::Error::from)
    }

    fn Revert(&self) -> windows::core::Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn LockRegion(
        &self,
        _offset: u64,
        _bytes: u64,
        _kind: &windows::Win32::System::Com::LOCKTYPE,
    ) -> windows::core::Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn UnlockRegion(&self, _offset: u64, _bytes: u64, _kind: u32) -> windows::core::Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn Stat(&self, stat: *mut STATSTG, _flags: &STATFLAG) -> windows::core::Result<()> {
        if stat.is_null() {
            return Err(E_FAIL.into());
        }
        let metadata = HandleBackedStream::io(self.locked_file().metadata())?;
        unsafe {
            stat.write(STATSTG {
                cbSize: metadata.len(),
                grfMode: STGM_READWRITE,
                ..Default::default()
            })
        };
        Ok(())
    }

    fn Clone(&self) -> windows::core::Result<IStream> {
        let cursor = *self.locked_cursor();
        Ok(HandleBackedStream::new(self.file.clone(), cursor).into())
    }
}

impl IAgileObject_Impl for HandleBackedStream_Impl {}
