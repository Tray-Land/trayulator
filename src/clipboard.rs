//! Plain-text clipboard access. Data set here stays on the clipboard after the app exits.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// Puts `text` on the clipboard on behalf of `owner`, a window of ours: with no owner, emptying
/// the clipboard leaves it ownerless and setting data then fails. False when another app has it
/// locked; the caller still has the text, so this is never fatal.
pub fn set_text(owner: HWND, text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        // The text is ready before the clipboard is opened, so it's only emptied when there's
        // something to replace its contents with.
        let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * size_of::<u16>()) else {
            return false;
        };
        let target = GlobalLock(memory).cast::<u16>();
        if target.is_null() {
            let _ = GlobalFree(Some(memory));
            return false;
        }
        target.copy_from_nonoverlapping(wide.as_ptr(), wide.len());
        let _ = GlobalUnlock(memory);

        if OpenClipboard(Some(owner)).is_err() {
            let _ = GlobalFree(Some(memory));
            return false;
        }
        let copied = EmptyClipboard().is_ok() && SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(memory.0))).is_ok();
        let _ = CloseClipboard();

        // On success the system owns the memory; on failure it's still ours to free.
        if !copied {
            let _ = GlobalFree(Some(memory));
        }
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
