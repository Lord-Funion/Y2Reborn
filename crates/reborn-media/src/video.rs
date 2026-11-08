//! Safe, single-owner video decoding over the same loaded FFmpeg membrane as music.
use super::{api, error, text, Cancel, MediaApi};
use reborn_core::video::fit;
use std::{
    ffi::{c_char, c_int, c_void, CString},
    path::Path,
    ptr::NonNull,
};

#[repr(C)]
struct RawInfo {
    duration_ms: u64,
    width: u32,
    height: u32,
    sar_num: u32,
    sar_den: u32,
    codec: [c_char; 64],
}
type Open =
    unsafe extern "C" fn(*const c_char, *mut c_void, *mut *mut c_void, *mut RawInfo) -> c_int;
type Read = unsafe extern "C" fn(*mut c_void, *mut u8, c_int, c_int, *mut i64) -> c_int;
type Seek = unsafe extern "C" fn(*mut c_void, i64) -> c_int;
type Close = unsafe extern "C" fn(*mut c_void);

#[derive(Clone, Debug)]
pub struct VideoInfo {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub display_width: u32,
    pub display_height: u32,
    pub codec: String,
}

#[derive(Debug)]
pub struct VideoFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub position_ms: u64,
}

pub struct VideoDecoder {
    raw: NonNull<c_void>,
    api: &'static MediaApi,
    read: Read,
    seek: Seek,
    close: Close,
    _cancel: Cancel,
    pub info: VideoInfo,
}

impl VideoDecoder {
    /// Opens a regular local file, including a storage guard's pinned descriptor path.
    pub fn open(path: &Path, cancel: Cancel) -> Result<Self, String> {
        use std::os::unix::ffi::OsStrExt;
        if !path.is_file() {
            return Err("video source is missing".into());
        }
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| "NUL in path")?;
        let api = api()?;
        let mut ptr = std::ptr::null_mut();
        let mut info = RawInfo {
            duration_ms: 0,
            width: 0,
            height: 0,
            sar_num: 0,
            sar_den: 0,
            codec: [0; 64],
        };
        // SAFETY: typed symbols from the process-lifetime membrane; checked local filename,
        // POD outputs and cancellation allocation remain live for the entire native ownership.
        let (read, seek, close) = unsafe {
            let open: Open = MediaApi::symbol(api._handle, b"rb_video_open\0")?;
            let read = MediaApi::symbol(api._handle, b"rb_video_read\0")?;
            let seek = MediaApi::symbol(api._handle, b"rb_video_seek\0")?;
            let close = MediaApi::symbol(api._handle, b"rb_video_close\0")?;
            let result = open(path.as_ptr(), cancel.0.raw.as_ptr(), &mut ptr, &mut info);
            if result < 0 {
                return Err(format!("video could not open: {}", error(api, result)));
            }
            (read, seek, close)
        };
        let (display_width, display_height) =
            fit(info.width, info.height, info.sar_num, info.sar_den);
        Ok(Self {
            raw: NonNull::new(ptr).ok_or("missing video context")?,
            api,
            read,
            seek,
            close,
            _cancel: cancel,
            info: VideoInfo {
                duration_ms: info.duration_ms,
                width: info.width,
                height: info.height,
                display_width,
                display_height,
                codec: text(&info.codec),
            },
        })
    }

    pub fn seek(&mut self, position_ms: u64) -> Result<(), String> {
        let ms = i64::try_from(position_ms).map_err(|_| "video seek overflow")?;
        // SAFETY: the exclusively borrowed context remains owned by self.
        let result = unsafe { (self.seek)(self.raw.as_ptr(), ms) };
        if result < 0 {
            Err(error(self.api, result))
        } else {
            Ok(())
        }
    }

    pub fn read(&mut self) -> Result<Option<VideoFrame>, String> {
        let (width, height) = (self.info.display_width, self.info.display_height);
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(width as usize * height as usize * 4)
            .map_err(|_| "not enough memory for a video picture")?;
        rgba.resize(width as usize * height as usize * 4, 0);
        let mut position_ms = 0;
        // SAFETY: output has exactly the checked RGBA dimensions; self exclusively owns the decoder.
        let result = unsafe {
            (self.read)(
                self.raw.as_ptr(),
                rgba.as_mut_ptr(),
                width as c_int,
                height as c_int,
                &mut position_ms,
            )
        };
        match result {
            0 => Ok(None),
            n if n < 0 => Err(format!("video decode failed: {}", error(self.api, n))),
            _ => Ok(Some(VideoFrame {
                rgba,
                width,
                height,
                position_ms: position_ms.max(0) as u64,
            })),
        }
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        // SAFETY: sole native owner, closed once before releasing its cancellation allocation.
        unsafe { (self.close)(self.raw.as_ptr()) };
    }
}
