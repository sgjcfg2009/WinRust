use std::{cell::RefCell, ffi::c_void, os::windows::{ffi::OsStrExt, process::CommandExt}, path::{Path, PathBuf}, sync::Mutex};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::{Com::*, Ole::*, Variant::*},
        UI::{Controls::*, Shell::Common::ITEMIDLIST, Shell::*, WindowsAndMessaging::*},
    },
};

pub const WM_SHELL_SELECT: u32 = WM_APP + 6;

struct Reg { sw: IShellWindows, cookie: i32, live: bool }
thread_local! { static REG: RefCell<Vec<Reg>> = const { RefCell::new(Vec::new()) }; }
static DIR: Mutex<String> = Mutex::new(String::new());

fn nope<T>() -> Result<T> { Err(E_NOTIMPL.into()) }
fn url(dir: &str) -> String { format!("file:///{}", cur(dir).replace('\\', "/")) }
fn cur(dir: &str) -> String { if dir.is_empty() { DIR.lock().unwrap().clone() } else { dir.to_string() } }
fn wide(s: &std::ffi::OsStr) -> Vec<u16> { s.encode_wide().chain([0]).collect() }

fn pidl_variant(dir: &Path) -> Option<(VARIANT, *mut ITEMIDLIST)> {
    let w = wide(dir.as_os_str());
    unsafe {
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).ok()?;
        InitVariantFromBuffer(pidl as *const c_void, ILGetSize(Some(pidl))).ok().map(|v| (v, pidl))
    }
}

fn add(hwnd: isize, dir: &Path, resident: bool) -> Result<()> {
    unsafe {
        let sw: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let me: IWebBrowser2 = Browser { hwnd, dir: if resident { dir.to_string_lossy().into_owned() } else { String::new() }, resident }.into();
        let cookie = sw.Register(&me.cast::<IDispatch>()?, hwnd as i32, SWC_BROWSER)?;
        if let Some((v, pidl)) = pidl_variant(dir) {
            let _ = sw.OnNavigate(cookie, &v);
            CoTaskMemFree(Some(pidl as *const c_void));
        }
        REG.with_borrow_mut(|r| r.push(Reg { sw, cookie, live: !resident }));
    }
    Ok(())
}

pub fn register(hwnd: isize) { let _ = add(hwnd, Path::new(&DIR.lock().unwrap().clone()), false); }

pub fn register_resident(dirs: &[PathBuf]) {
    let h = unsafe { CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, w!("STATIC"), w!(""), WS_POPUP, 0, 0, 0, 0, None, None, None, None) };
    if let Ok(h) = h { for d in dirs { let _ = add(h.0 as isize, d, true); } }
}

pub fn navigated(dir: &Path) {
    *DIR.lock().unwrap() = dir.to_string_lossy().into_owned();
    REG.with_borrow(|rs| {
        for r in rs.iter().filter(|r| r.live) {
            if let Some((v, pidl)) = pidl_variant(dir) {
                unsafe { let _ = r.sw.OnNavigate(r.cookie, &v); CoTaskMemFree(Some(pidl as *const c_void)); }
            }
        }
    });
}

pub fn revoke() {
    for r in REG.take() { unsafe { let _ = r.sw.Revoke(r.cookie); } }
}

#[implement(IWebBrowser2)]
struct Browser { hwnd: isize, dir: String, resident: bool }
impl IDispatch_Impl for Browser_Impl {
    fn GetTypeInfoCount(&self) -> Result<u32> { Ok(0) }
    fn GetTypeInfo(&self, _: u32, _: u32) -> Result<ITypeInfo> { nope() }
    fn GetIDsOfNames(&self, _: *const GUID, _: *const PCWSTR, _: u32, _: u32, _: *mut i32) -> Result<()> { nope() }
    fn Invoke(&self, _: i32, _: *const GUID, _: u32, _: DISPATCH_FLAGS, _: *const DISPPARAMS, _: *mut VARIANT, _: *mut EXCEPINFO, _: *mut u32) -> Result<()> { nope() }
}

#[implement(IDispatch, IServiceProvider)]
struct Doc { hwnd: isize, dir: String, resident: bool }
impl IDispatch_Impl for Doc_Impl {
    fn GetTypeInfoCount(&self) -> Result<u32> { Ok(0) }
    fn GetTypeInfo(&self, _: u32, _: u32) -> Result<ITypeInfo> { nope() }
    fn GetIDsOfNames(&self, _: *const GUID, _: *const PCWSTR, _: u32, _: u32, _: *mut i32) -> Result<()> { nope() }
    fn Invoke(&self, _: i32, _: *const GUID, _: u32, _: DISPATCH_FLAGS, _: *const DISPPARAMS, _: *mut VARIANT, _: *mut EXCEPINFO, _: *mut u32) -> Result<()> { nope() }
}
impl IServiceProvider_Impl for Doc_Impl {
    fn QueryService(&self, _svc: *const GUID, riid: *const GUID, out: *mut *mut c_void) -> Result<()> {
       
        let v: IShellView = View { hwnd: self.hwnd, dir: self.dir.clone(), resident: self.resident }.into();
        unsafe { v.query(&*riid, out).ok() }
    }
}

#[implement(IShellView)]
struct View { hwnd: isize, dir: String, resident: bool }
impl IOleWindow_Impl for View_Impl {
    fn GetWindow(&self) -> Result<HWND> { Ok(HWND(self.hwnd as *mut c_void)) }
    fn ContextSensitiveHelp(&self, _: BOOL) -> Result<()> { Ok(()) }
}
impl IShellView_Impl for View_Impl {
    fn TranslateAccelerator(&self, _: *const MSG) -> HRESULT { E_NOTIMPL }
    fn EnableModeless(&self, _: BOOL) -> Result<()> { Ok(()) }
    fn UIActivate(&self, _: u32) -> Result<()> { Ok(()) }
    fn Refresh(&self) -> Result<()> { Ok(()) }
    fn CreateViewWindow(&self, _: Ref<IShellView>, _: *const FOLDERSETTINGS, _: Ref<IShellBrowser>, _: *const RECT) -> Result<HWND> { nope() }
    fn DestroyViewWindow(&self) -> Result<()> { Ok(()) }
    fn GetCurrentInfo(&self) -> Result<FOLDERSETTINGS> { nope() }
    fn AddPropertySheetPages(&self, _: u32, _: LPFNSVADDPROPSHEETPAGE, _: LPARAM) -> Result<()> { nope() }
    fn SaveViewState(&self) -> Result<()> { Ok(()) }
    fn SelectItem(&self, pidl: *const ITEMIDLIST, _flags: u32) -> Result<()> {
       
        let path = item_path(pidl, &cur(&self.dir)).ok_or(E_FAIL)?;
        if self.resident {
            let exe = std::env::current_exe().map_err(|_| E_FAIL)?;
            std::process::Command::new(exe).raw_arg(format!("/select,\"{}\"", path.display())).spawn().map_err(|_| E_FAIL)?;
        } else {
            unsafe { PostMessageW(Some(HWND(self.hwnd as *mut c_void)), WM_SHELL_SELECT, WPARAM(0), LPARAM(Box::into_raw(Box::new(path)) as isize))? };
        }
        Ok(())
    }
    fn GetItemObject(&self, _: &_SVGIO, _: *const GUID, _: *mut *mut c_void) -> Result<()> { nope() }
}

fn item_path(pidl: *const ITEMIDLIST, dir: &str) -> Option<PathBuf> {
    let name = |p: *const ITEMIDLIST| unsafe {
        let s = SHGetNameFromIDList(p, SIGDN_FILESYSPATH).ok()?;
        let r = s.to_string().ok();
        CoTaskMemFree(Some(s.0 as *const c_void));
        r.map(PathBuf::from)
    };
    let w = wide(std::ffi::OsStr::new(dir));
    let joined = unsafe {
        let mut base = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut base, 0, None).ok().and_then(|_| {
            let abs = ILCombine(Some(base), Some(pidl));
            let p = name(abs);
            ILFree(Some(abs));
            CoTaskMemFree(Some(base as *const c_void));
            p
        })
    };
    joined.filter(|p| p.parent().is_some_and(|d| d.as_os_str().eq_ignore_ascii_case(Path::new(dir).as_os_str()))).or_else(|| name(pidl))
}

impl IWebBrowser_Impl for Browser_Impl {
    fn GoBack(&self) -> Result<()> { nope() }
    fn GoForward(&self) -> Result<()> { nope() }
    fn GoHome(&self) -> Result<()> { nope() }
    fn GoSearch(&self) -> Result<()> { nope() }
    fn Navigate(&self, _url: &BSTR, _flags: *const windows::Win32::System::Variant::VARIANT, _targetframename: *const windows::Win32::System::Variant::VARIANT, _postdata: *const windows::Win32::System::Variant::VARIANT, _headers: *const windows::Win32::System::Variant::VARIANT) -> Result<()> { nope() }
    fn Refresh(&self) -> Result<()> { nope() }
    fn Refresh2(&self, _level: *const windows::Win32::System::Variant::VARIANT) -> Result<()> { nope() }
    fn Stop(&self) -> Result<()> { nope() }
    fn Application(&self) -> Result<windows::Win32::System::Com::IDispatch> { nope() }
    fn Parent(&self) -> Result<windows::Win32::System::Com::IDispatch> { nope() }
    fn Container(&self) -> Result<windows::Win32::System::Com::IDispatch> { nope() }
    fn Document(&self) -> Result<windows::Win32::System::Com::IDispatch> { Ok(Doc { hwnd: self.hwnd, dir: self.dir.clone(), resident: self.resident }.into()) }
    fn TopLevelContainer(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn Type(&self) -> Result<BSTR> { nope() }
    fn Left(&self) -> Result<i32> { nope() }
    fn SetLeft(&self, _left: i32) -> Result<()> { nope() }
    fn Top(&self) -> Result<i32> { nope() }
    fn SetTop(&self, _top: i32) -> Result<()> { nope() }
    fn Width(&self) -> Result<i32> { nope() }
    fn SetWidth(&self, _width: i32) -> Result<()> { nope() }
    fn Height(&self) -> Result<i32> { nope() }
    fn SetHeight(&self, _height: i32) -> Result<()> { nope() }
    fn LocationName(&self) -> Result<BSTR> { Ok(BSTR::from(url(&self.dir))) }
    fn LocationURL(&self) -> Result<BSTR> { Ok(BSTR::from(url(&self.dir))) }
    fn Busy(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
}
impl IWebBrowserApp_Impl for Browser_Impl {
    fn Quit(&self) -> Result<()> { nope() }
    fn ClientToWindow(&self, _pcx: *mut i32, _pcy: *mut i32) -> Result<()> { nope() }
    fn PutProperty(&self, _property: &BSTR, _vtvalue: &windows::Win32::System::Variant::VARIANT) -> Result<()> { nope() }
    fn GetProperty(&self, _property: &BSTR) -> Result<windows::Win32::System::Variant::VARIANT> { nope() }
    fn Name(&self) -> Result<BSTR> { nope() }
    fn HWND(&self) -> Result<windows::Win32::Foundation::SHANDLE_PTR> { Ok(SHANDLE_PTR(self.hwnd)) }
    fn FullName(&self) -> Result<BSTR> { nope() }
    fn Path(&self) -> Result<BSTR> { nope() }
    fn Visible(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { Ok(VARIANT_TRUE) }
    fn SetVisible(&self, _value: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
    fn StatusBar(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn SetStatusBar(&self, _value: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
    fn StatusText(&self) -> Result<BSTR> { nope() }
    fn SetStatusText(&self, _statustext: &BSTR) -> Result<()> { nope() }
    fn ToolBar(&self) -> Result<i32> { nope() }
    fn SetToolBar(&self, _value: i32) -> Result<()> { nope() }
    fn MenuBar(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn SetMenuBar(&self, _value: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
    fn FullScreen(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn SetFullScreen(&self, _bfullscreen: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
}
impl IWebBrowser2_Impl for Browser_Impl {
    fn Navigate2(&self, _url: *const windows::Win32::System::Variant::VARIANT, _flags: *const windows::Win32::System::Variant::VARIANT, _targetframename: *const windows::Win32::System::Variant::VARIANT, _postdata: *const windows::Win32::System::Variant::VARIANT, _headers: *const windows::Win32::System::Variant::VARIANT) -> Result<()> { nope() }
    fn QueryStatusWB(&self, _cmdid: windows::Win32::System::Ole::OLECMDID) -> Result<windows::Win32::System::Ole::OLECMDF> { nope() }
    fn ExecWB(&self, _cmdid: windows::Win32::System::Ole::OLECMDID, _cmdexecopt: windows::Win32::System::Ole::OLECMDEXECOPT, _pvain: *const windows::Win32::System::Variant::VARIANT, _pvaout: *mut windows::Win32::System::Variant::VARIANT) -> Result<()> { nope() }
    fn ShowBrowserBar(&self, _pvaclsid: *const windows::Win32::System::Variant::VARIANT, _pvarshow: *const windows::Win32::System::Variant::VARIANT, _pvarsize: *const windows::Win32::System::Variant::VARIANT) -> Result<()> { nope() }
    fn ReadyState(&self) -> Result<windows::Win32::System::Ole::READYSTATE> { nope() }
    fn Offline(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn SetOffline(&self, _boffline: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
    fn Silent(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn SetSilent(&self, _bsilent: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
    fn RegisterAsBrowser(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn SetRegisterAsBrowser(&self, _bregister: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
    fn RegisterAsDropTarget(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn SetRegisterAsDropTarget(&self, _bregister: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
    fn TheaterMode(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn SetTheaterMode(&self, _bregister: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
    fn AddressBar(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn SetAddressBar(&self, _value: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
    fn Resizable(&self) -> Result<windows::Win32::Foundation::VARIANT_BOOL> { nope() }
    fn SetResizable(&self, _value: windows::Win32::Foundation::VARIANT_BOOL) -> Result<()> { nope() }
}
