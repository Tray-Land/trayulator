//! Plain-text clipboard access. Data set here stays on the clipboard after the app exits.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// Puts `text` on the clipboard. False when another app has it locked; the caller still has the
/// text, so this is never fatal.
pub fn set_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }

        let copied = (|| -> windows::core::Result<()> {
            EmptyClipboard()?;
            let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * size_of::<u16>())?;
            let target = GlobalLock(memory).cast::<u16>();
            if target.is_null() {
                let _ = GlobalFree(Some(memory));
                return Err(windows::core::Error::from_thread());
            }
            target.copy_from_nonoverlapping(wide.as_ptr(), wide.len());
            let _ = GlobalUnlock(memory);

            // On success the system owns the memory; on failure it's still ours to free.
            if let Err(error) = SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(memory.0))) {
                let _ = GlobalFree(Some(memory));
                return Err(error);
            }
            Ok(())
        })()
        .is_ok();

        let _ = CloseClipboard();
        copied
    }
}

/// The clipboard's text, or `None` when it holds none or can't be opened.
pub fn text() -> Option<String> {
    unsafe {
        OpenClipboard(None).ok()?;
        let text = GetClipboardData(u32::from(CF_UNICODETEXT.0)).ok().and_then(|data| {
            let memory = HGLOBAL(data.0);
            let source = GlobalLock(memory).cast::<u16>();
            if source.is_null() {
                return None;
            }
            let length = (0..).take_while(|&i| *source.add(i) != 0).count();
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(source, length));
            let _ = GlobalUnlock(memory);
            Some(text)
        });

        let _ = CloseClipboard();
        text
    }
}
