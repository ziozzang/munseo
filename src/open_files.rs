//! Finder 의 "다음으로 열기"·Dock 아이콘에 놓기로 넘어오는 파일을 받는다 (macOS).
//!
//! AppKit 은 이런 파일을 앱 델리게이트의 `application:openURLs:` 로 보낸다. winit 0.30 은 자기
//! 델리게이트(WinitApplicationDelegate)를 등록하면서 이 메서드를 구현하지 않고, 이벤트 루프를 만든 뒤
//! 우리 코드가 끼어들 틈도 없다. 그래서 이벤트 루프를 만들기 전에 NSObject 에 이 메서드를 더해 둔다.
//! winit 델리게이트는 NSObject 를 상속하므로 이 구현을 물려받고, 앱 실행 직후에 오는 파일도 놓치지 않는다.

use std::ffi::{c_char, CStr};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use eframe::egui;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{msg_send, sel};

static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// 이벤트 루프를 만들기 전에(= `eframe::run_native` 전에) 한 번 부른다.
pub fn install() {
    let Some(class) = AnyClass::get(c"NSObject") else { return };
    // SAFETY: 타입 인코딩 "v@:@@" 는 아래 함수 시그니처 (void, self, _cmd, NSApplication*, NSArray*) 와 같다.
    // 같은 이름의 메서드가 이미 있으면 class_addMethod 는 아무것도 바꾸지 않는다.
    unsafe {
        let imp: Imp = std::mem::transmute(
            open_urls as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
        );
        objc2::ffi::class_addMethod(
            class as *const AnyClass as *mut AnyClass,
            sel!(application:openURLs:),
            imp,
            c"v@:@@".as_ptr(),
        );
    }
}

/// 창이 준비되면 넘겨받아, 파일이 들어올 때 다시 그리게 한다.
pub fn set_context(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// 지금까지 들어온 파일을 꺼낸다.
pub fn take() -> Vec<PathBuf> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
}

unsafe extern "C-unwind" fn open_urls(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, urls: *mut AnyObject) {
    let Some(urls) = (unsafe { urls.as_ref() }) else { return };
    let mut paths = Vec::new();
    let count: usize = unsafe { msg_send![urls, count] };
    for i in 0..count {
        let url: *mut AnyObject = unsafe { msg_send![urls, objectAtIndex: i] };
        let Some(url) = (unsafe { url.as_ref() }) else { continue };
        let path: *mut AnyObject = unsafe { msg_send![url, path] };
        let Some(path) = (unsafe { path.as_ref() }) else { continue };
        let utf8: *const c_char = unsafe { msg_send![path, UTF8String] };
        if !utf8.is_null() {
            paths.push(PathBuf::from(unsafe { CStr::from_ptr(utf8) }.to_string_lossy().into_owned()));
        }
    }
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).extend(paths);
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}
