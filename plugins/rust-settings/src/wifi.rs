use std::ptr::{null, null_mut};
use windows_sys::{
    Win32::{Foundation::*, NetworkManagement::WiFi::*, UI::WindowsAndMessaging::PostMessageW},
    core::GUID,
};

pub const WM_WIFI: u32 = 0x8000 + 1;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sec {
    Open,
    Owe,
    Wep { shared: bool },
    Wpa { tkip: bool },
    Wpa2 { tkip: bool },
    Wpa3,
}

impl Sec {
    fn of(secured: bool, auth: i32, cipher: i32) -> Option<Sec> {
        let tkip = cipher == DOT11_CIPHER_ALGO_TKIP;
        let wep = matches!(cipher, DOT11_CIPHER_ALGO_WEP | DOT11_CIPHER_ALGO_WEP40 | DOT11_CIPHER_ALGO_WEP104);
        Some(match auth {
            DOT11_AUTH_ALGO_80211_OPEN if !secured => Sec::Open,
            DOT11_AUTH_ALGO_80211_OPEN if wep => Sec::Wep { shared: false },
            DOT11_AUTH_ALGO_80211_SHARED_KEY => Sec::Wep { shared: true },
            DOT11_AUTH_ALGO_OWE => Sec::Owe,
            DOT11_AUTH_ALGO_WPA_PSK => Sec::Wpa { tkip },
            DOT11_AUTH_ALGO_RSNA_PSK => Sec::Wpa2 { tkip },
            DOT11_AUTH_ALGO_WPA3_SAE => Sec::Wpa3,
            _ => return None,
        })
    }

    pub fn needs_key(self) -> bool {
        !matches!(self, Sec::Open | Sec::Owe)
    }

    fn names(self) -> (&'static str, &'static str) {
        let enc = |tkip| if tkip { "TKIP" } else { "AES" };
        match self {
            Sec::Open => ("open", "none"),
            Sec::Owe => ("OWE", "AES"),
            Sec::Wep { shared } => (if shared { "shared" } else { "open" }, "WEP"),
            Sec::Wpa { tkip } => ("WPAPSK", enc(tkip)),
            Sec::Wpa2 { tkip } => ("WPA2PSK", enc(tkip)),
            Sec::Wpa3 => ("WPA3SAE", "AES"),
        }
    }

    fn key_type(self, key: &str) -> Result<&'static str, String> {
        let hex = |n: usize| key.len() == n && key.bytes().all(|b| b.is_ascii_hexdigit());
        match self {
            Sec::Open | Sec::Owe => Ok(""),
            Sec::Wep { .. } if [5, 13].contains(&key.len()) && key.is_ascii() || hex(10) || hex(26) => Ok("networkKey"),
            Sec::Wep { .. } => Err("WEP key: 5 or 13 characters (or 10/26 hex digits)".into()),
            Sec::Wpa { .. } | Sec::Wpa2 { .. } if hex(64) => Ok("networkKey"),
            _ if (8..=63).contains(&key.len()) => Ok("passPhrase"),
            _ => Err("Password: 8 to 63 characters".into()),
        }
    }
}

#[derive(Clone)]
pub struct Net {
    pub ssid: String,
    pub raw: Vec<u8>,
    pub quality: u32,
    pub connected: bool,
    pub profile: Option<String>,
    pub sec: Option<Sec>,
    pub eap: Option<&'static str>,
    pub iface: GUID,
}

pub fn same_iface(a: &GUID, b: &GUID) -> bool {
    (a.data1, a.data2, a.data3, a.data4) == (b.data1, b.data2, b.data3, b.data4)
}

fn eap_of(secured: bool, auth: i32) -> Option<&'static str> {
    match auth {
        DOT11_AUTH_ALGO_RSNA if secured => Some("WPA2"),
        DOT11_AUTH_ALGO_WPA if secured => Some("WPA"),
        _ => None,
    }
}

struct Client(HANDLE);

impl Drop for Client {
    fn drop(&mut self) {
        unsafe { WlanCloseHandle(self.0, null()) };
    }
}

fn open_all() -> Option<(Client, Vec<GUID>)> {
    let (mut ver, mut h) = (0u32, null_mut());
    if unsafe { WlanOpenHandle(WLAN_API_VERSION_2_0, null(), &mut ver, &mut h) } != 0 {
        return None;
    }
    let c = Client(h);
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
    if unsafe { WlanEnumInterfaces(h, null(), &mut list) } != 0 || list.is_null() {
        return None;
    }
    let guids: Vec<GUID> = unsafe { std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), (*list).dwNumberOfItems as usize).iter().map(|i| i.InterfaceGuid).collect() };
    unsafe { WlanFreeMemory(list as _) };
    (!guids.is_empty()).then_some((c, guids))
}

fn utf16(s: &[u16]) -> String {
    String::from_utf16_lossy(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())])
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn tidy(v: Vec<Net>) -> Vec<Net> {
    let mut out: Vec<Net> = vec![];
    for n in v.into_iter().filter(|n| !n.raw.is_empty()) {
        match out.iter_mut().find(|o| o.raw == n.raw) {
            Some(o) => {
                if n.connected || (!o.connected && n.quality > o.quality) {
                    o.iface = n.iface;
                }
                o.quality = o.quality.max(n.quality);
                o.connected |= n.connected;
                if o.profile.is_none() {
                    o.profile = n.profile;
                }
            }
            None => out.push(n),
        }
    }
    out.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.quality.cmp(&a.quality)));
    out
}

pub fn scan() {
    if let Some((c, guids)) = open_all() {
        for g in guids {
            unsafe { WlanScan(c.0, &g, null(), null(), null()) };
        }
    }
}

pub fn list() -> Vec<Net> {
    let Some((c, guids)) = open_all() else { return vec![] };
    let mut out = vec![];
    for guid in guids {
        let mut nl: *mut WLAN_AVAILABLE_NETWORK_LIST = null_mut();
        if unsafe { WlanGetAvailableNetworkList(c.0, &guid, 0, null(), &mut nl) } != 0 || nl.is_null() {
            continue;
        }
        let items = unsafe { std::slice::from_raw_parts((*nl).Network.as_ptr(), (*nl).dwNumberOfItems as usize) };
        out.extend(items.iter().filter(|n| n.dot11BssType == dot11_BSS_type_infrastructure).map(|n| {
            let raw = n.dot11Ssid.ucSSID[..(n.dot11Ssid.uSSIDLength as usize).min(32)].to_vec();
            Net {
                ssid: String::from_utf8_lossy(&raw).into_owned(),
                raw,
                quality: n.wlanSignalQuality,
                connected: n.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
                profile: (n.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0).then(|| utf16(&n.strProfileName)),
                sec: Sec::of(n.bSecurityEnabled != 0, n.dot11DefaultAuthAlgorithm, n.dot11DefaultCipherAlgorithm),
                eap: eap_of(n.bSecurityEnabled != 0, n.dot11DefaultAuthAlgorithm),
                iface: guid,
            }
        }));
        unsafe { WlanFreeMemory(nl as _) };
    }
    tidy(out)
}

pub fn radio() -> Option<(bool, bool)> {
    let (c, guids) = open_all()?;
    let (mut soft, mut hard, mut any) = (false, true, false);
    for guid in guids {
        let (mut n, mut p, mut ty) = (0u32, null_mut(), 0);
        if unsafe { WlanQueryInterface(c.0, &guid, wlan_intf_opcode_radio_state, null(), &mut n, &mut p, &mut ty) } != 0 || p.is_null() {
            continue;
        }
        let st = unsafe { &*(p as *const WLAN_RADIO_STATE) };
        let phys = &st.PhyRadioState[..(st.dwNumberOfPhys as usize).min(64)];
        soft |= phys.iter().any(|x| x.dot11SoftwareRadioState == dot11_radio_state_on);
        hard &= phys.iter().all(|x| x.dot11HardwareRadioState == dot11_radio_state_on);
        any = true;
        unsafe { WlanFreeMemory(p) };
    }
    any.then_some((soft, hard))
}

pub fn set_radio(on: bool) {
    std::thread::spawn(move || {
        use windows::Devices::Radios::{Radio, RadioKind, RadioState};
        let _ = Radio::RequestAccessAsync().and_then(|a| a.join());
        let Ok(radios) = Radio::GetRadiosAsync().and_then(|a| a.join()) else { return };
        for r in radios.into_iter().filter(|r| r.Kind().is_ok_and(|k| k == RadioKind::WiFi)) {
            let _ = r.SetStateAsync(if on { RadioState::On } else { RadioState::Off }).and_then(|a| a.join());
        }
    });
}

pub fn connect(profile: &str, guid: &GUID) -> bool {
    let Some((c, _)) = open_all() else { return false };
    let name = wide(profile);
    let p = WLAN_CONNECTION_PARAMETERS { wlanConnectionMode: wlan_connection_mode_profile, strProfile: name.as_ptr(), pDot11Ssid: null_mut(), pDesiredBssidList: null_mut(), dot11BssType: dot11_BSS_type_infrastructure, dwFlags: 0 };
    unsafe { WlanConnect(c.0, guid, &p, null()) == 0 }
}

pub fn disconnect() -> bool {
    open_all().is_some_and(|(c, guids)| guids.iter().fold(false, |any, g| unsafe { WlanDisconnect(c.0, g, null()) == 0 } || any))
}

pub fn forget(profile: &str) -> bool {
    open_all().is_some_and(|(c, guids)| guids.iter().fold(false, |any, g| unsafe { WlanDeleteProfile(c.0, g, wide(profile).as_ptr(), null()) == 0 } || any))
}

pub fn reason_text(code: u32) -> String {
    let mut buf = [0u16; 512];
    if unsafe { WlanReasonCodeToString(code, buf.len() as u32, buf.as_mut_ptr(), null()) } == 0 {
        let s = utf16(&buf);
        if !s.is_empty() {
            return s;
        }
    }
    format!("code {}", code)
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn hex_bytes(raw: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(raw.len() * 2);
    for &b in raw {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xF) as usize] as char);
    }
    out
}

fn profile_xml(name: &str, raw: &[u8], sec: Sec, key: &str, key_type: &str, hidden: bool) -> String {
    let hex = hex_bytes(raw);
    let (auth, enc) = sec.names();
    let name = xml_escape(name);
    let hidden = if hidden { "<nonBroadcast>true</nonBroadcast>" } else { "" };
    let shared = if sec.needs_key() {
        let idx = if matches!(sec, Sec::Wep { .. }) { "<keyIndex>0</keyIndex>" } else { "" };
        format!("<sharedKey><keyType>{key_type}</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>{idx}", xml_escape(key))
    } else {
        String::new()
    };
    format!(
        r#"<?xml version="1.0"?><WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1"><name>{name}</name><SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID>{hidden}</SSIDConfig><connectionType>ESS</connectionType><connectionMode>auto</connectionMode><MSM><security><authEncryption><authentication>{auth}</authentication><encryption>{enc}</encryption><useOneX>false</useOneX></authEncryption>{shared}</security></MSM></WLANProfile>"#
    )
}

pub fn add_and_connect(ssid: &str, raw: &[u8], sec: Sec, key: &str, hidden: bool, iface: Option<&GUID>) -> Result<String, String> {
    let kt = sec.key_type(key)?;
    let Some((c, guid)) = pick(iface) else { return Err("No Wi-Fi adapter".into()) };
    let mut xml = wide(&profile_xml(ssid, raw, sec, key, kt, hidden));
    let mut reason = 0u32;
    let rc = unsafe { WlanSetProfile(c.0, &guid, 0, xml.as_ptr(), null(), 1, null(), &mut reason) };
    xml.iter_mut().for_each(|w| *w = 0);
    if rc != 0 {
        return Err(if rc == ERROR_ACCESS_DENIED { "Windows refused to save networks (policy)".into() } else { format!("Profile rejected: {}", reason_text(reason)) });
    }
    if !connect(ssid, &guid) {
        forget(ssid);
        return Err("Could not start connecting".into());
    }
    Ok(ssid.to_string())
}

fn pick(iface: Option<&GUID>) -> Option<(Client, GUID)> {
    let (c, guids) = open_all()?;
    let g = iface.and_then(|w| guids.iter().find(|g| same_iface(g, w))).or(guids.first()).copied()?;
    Some((c, g))
}

fn eap_profile_xml(name: &str, raw: &[u8], auth: &str, hidden: bool) -> String {
    let hex = hex_bytes(raw);
    let name = xml_escape(name);
    let enc = if auth == "WPA" { "TKIP" } else { "AES" };
    let hidden = if hidden { "<nonBroadcast>true</nonBroadcast>" } else { "" };
    format!(
        r#"<?xml version="1.0"?><WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1"><name>{name}</name><SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID>{hidden}</SSIDConfig><connectionType>ESS</connectionType><connectionMode>auto</connectionMode><MSM><security><authEncryption><authentication>{auth}</authentication><encryption>{enc}</encryption><useOneX>true</useOneX></authEncryption><OneX xmlns="http://www.microsoft.com/networking/OneX/v1"><authMode>user</authMode><EAPConfig><EapHostConfig xmlns="http://www.microsoft.com/provisioning/EapHostConfig" xmlns:eapCommon="http://www.microsoft.com/provisioning/EapCommon" xmlns:baseEap="http://www.microsoft.com/provisioning/BaseEapConnectionPropertiesV1"><EapMethod><eapCommon:Type>25</eapCommon:Type><eapCommon:AuthorId>0</eapCommon:AuthorId></EapMethod><Config xmlns:baseEap="http://www.microsoft.com/provisioning/BaseEapConnectionPropertiesV1" xmlns:msPeap="http://www.microsoft.com/provisioning/MsPeapConnectionPropertiesV1" xmlns:msChapV2="http://www.microsoft.com/provisioning/MsChapV2ConnectionPropertiesV1"><baseEap:Eap><baseEap:Type>25</baseEap:Type><msPeap:EapType><msPeap:ServerValidation><msPeap:DisableUserPromptForServerValidation>true</msPeap:DisableUserPromptForServerValidation><msPeap:ServerNames></msPeap:ServerNames></msPeap:ServerValidation><msPeap:FastReconnect>true</msPeap:FastReconnect><msPeap:InnerEapOptional>false</msPeap:InnerEapOptional><baseEap:Eap><baseEap:Type>26</baseEap:Type><msChapV2:EapType><msChapV2:UseWinLogonCredentials>false</msChapV2:UseWinLogonCredentials></msChapV2:EapType></baseEap:Eap><msPeap:EnableQuarantineChecks>false</msPeap:EnableQuarantineChecks><msPeap:RequireCryptoBinding>false</msPeap:RequireCryptoBinding><msPeap:PeapExtensions/></msPeap:EapType></baseEap:Eap></Config></EapHostConfig></EAPConfig></OneX></security></MSM></WLANProfile>"#
    )
}

fn eap_user_xml(user: &str, pass: &str) -> String {
    let (domain, user) = user.split_once('\\').unwrap_or(("", user));
    let (d, u, p) = (xml_escape(domain), xml_escape(user), xml_escape(pass));
    format!(
        r#"<?xml version="1.0"?><EapHostUserCredentials xmlns="http://www.microsoft.com/provisioning/EapHostUserCredentials" xmlns:eapCommon="http://www.microsoft.com/provisioning/EapCommon" xmlns:baseEap="http://www.microsoft.com/provisioning/BaseEapMethodUserCredentials"><EapMethod><eapCommon:Type>25</eapCommon:Type><eapCommon:AuthorId>0</eapCommon:AuthorId></EapMethod><Credentials xmlns:eapUser="http://www.microsoft.com/provisioning/EapUserPropertiesV1" xmlns:baseEap="http://www.microsoft.com/provisioning/BaseEapUserPropertiesV1" xmlns:MsPeap="http://www.microsoft.com/provisioning/MsPeapUserPropertiesV1" xmlns:MsChapV2="http://www.microsoft.com/provisioning/MsChapV2UserPropertiesV1"><baseEap:Eap><baseEap:Type>25</baseEap:Type><MsPeap:EapType><baseEap:Eap><baseEap:Type>26</baseEap:Type><MsChapV2:EapType><MsChapV2:Username>{u}</MsChapV2:Username><MsChapV2:Password>{p}</MsChapV2:Password><MsChapV2:LogonDomain>{d}</MsChapV2:LogonDomain></MsChapV2:EapType></baseEap:Eap></MsPeap:EapType></baseEap:Eap></Credentials></EapHostUserCredentials>"#
    )
}

pub fn add_eap_and_connect(ssid: &str, raw: &[u8], auth: &str, user: &str, pass: &str, hidden: bool, iface: Option<&GUID>) -> Result<String, String> {
    let Some((c, guid)) = pick(iface) else { return Err("No Wi-Fi adapter".into()) };
    let xml = wide(&eap_profile_xml(ssid, raw, auth, hidden));
    let mut reason = 0u32;
    let rc = unsafe { WlanSetProfile(c.0, &guid, 0, xml.as_ptr(), null(), 1, null(), &mut reason) };
    if rc != 0 {
        return Err(if rc == ERROR_ACCESS_DENIED { "Windows refused to save networks (policy)".into() } else { format!("Profile rejected: {}", reason_text(reason)) });
    }
    let mut creds = wide(&eap_user_xml(user, pass));
    let rc = unsafe { WlanSetProfileEapXmlUserData(c.0, &guid, wide(ssid).as_ptr(), 0, creds.as_ptr(), null()) };
    creds.fill(0);
    if rc != 0 {
        forget(ssid);
        return Err(format!("Could not save the credentials: {}", reason_text(rc)));
    }
    if !connect(ssid, &guid) {
        forget(ssid);
        return Err("Could not start connecting".into());
    }
    Ok(ssid.to_string())
}

unsafe extern "system" fn on_event(d: *mut L2_NOTIFICATION_DATA, ctx: *mut core::ffi::c_void) {
    let d = unsafe { &*d };
    if d.NotificationSource != WLAN_NOTIFICATION_SOURCE_ACM {
        return;
    }
    let code = d.NotificationCode as i32;
    let lp = if code == wlan_notification_acm_connection_complete && !d.pData.is_null() && d.dwDataSize as usize >= size_of::<WLAN_CONNECTION_NOTIFICATION_DATA>() {
        let c = unsafe { &*(d.pData as *const WLAN_CONNECTION_NOTIFICATION_DATA) };
        Box::into_raw(Box::new((c.wlanReasonCode, utf16(&c.strProfileName)))) as isize
    } else if [wlan_notification_acm_scan_complete, wlan_notification_acm_scan_fail, wlan_notification_acm_disconnected, wlan_notification_acm_interface_arrival, wlan_notification_acm_interface_removal].contains(&code) {
        0
    } else {
        return;
    };
    if unsafe { PostMessageW(ctx as HWND, WM_WIFI, code as usize, lp) } == 0 && lp != 0 {
        drop(unsafe { Box::from_raw(lp as *mut (u32, String)) });
    }
}

pub fn watch(hwnd: HWND) {
    let (mut ver, mut h) = (0u32, null_mut());
    if unsafe { WlanOpenHandle(WLAN_API_VERSION_2_0, null(), &mut ver, &mut h) } == 0 {
        unsafe { WlanRegisterNotification(h, WLAN_NOTIFICATION_SOURCE_ACM, 1, Some(on_event), hwnd as _, null(), null_mut()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str, q: u32, c: bool, p: Option<&str>) -> Net {
        Net { ssid: s.into(), raw: s.as_bytes().to_vec(), quality: q, connected: c, profile: p.map(Into::into), sec: Some(Sec::Wpa2 { tkip: false }), eap: None, iface: unsafe { std::mem::zeroed() } }
    }

    #[test]
    fn duplicates_merge_and_the_connected_goes_first() {
        let v = tidy(vec![n("a", 40, false, Some("a")), n("a", 90, false, None), n("", 99, false, None), n("b", 20, true, Some("b")), n("b", 20, false, None), n("c", 60, false, None)]);
        let got: Vec<_> = v.iter().map(|x| (x.ssid.as_str(), x.quality, x.connected, x.profile.is_some())).collect();
        assert_eq!(got, [("b", 20, true, true), ("a", 90, false, true), ("c", 60, false, false)]);
    }

    #[test]
    fn the_eap_documents_carry_escaped_credentials_and_split_the_domain() {
        let u = eap_user_xml("CORP\\ann<", "p&w");
        assert!(u.contains("<MsChapV2:Username>ann&lt;</MsChapV2:Username>") && u.contains("<MsChapV2:LogonDomain>CORP</MsChapV2:LogonDomain>") && u.contains("<MsChapV2:Password>p&amp;w</MsChapV2:Password>"));
        assert!(eap_user_xml("bob", "x").contains("<MsChapV2:LogonDomain></MsChapV2:LogonDomain>"));
        let p = eap_profile_xml("corp", b"corp", "WPA2", false);
        assert!(p.contains("<useOneX>true</useOneX>") && p.contains("<authentication>WPA2</authentication>") && p.contains("<hex>636F7270</hex>"));
        assert_eq!(eap_of(true, DOT11_AUTH_ALGO_RSNA), Some("WPA2"));
        assert_eq!(eap_of(true, DOT11_AUTH_ALGO_RSNA_PSK), None);
    }

    #[test]
    fn security_is_read_from_auth_and_cipher() {
        assert_eq!(Sec::of(false, DOT11_AUTH_ALGO_80211_OPEN, DOT11_CIPHER_ALGO_NONE), Some(Sec::Open));
        assert_eq!(Sec::of(true, DOT11_AUTH_ALGO_80211_OPEN, DOT11_CIPHER_ALGO_WEP), Some(Sec::Wep { shared: false }));
        assert_eq!(Sec::of(true, DOT11_AUTH_ALGO_RSNA_PSK, DOT11_CIPHER_ALGO_CCMP), Some(Sec::Wpa2 { tkip: false }));
        assert_eq!(Sec::of(true, DOT11_AUTH_ALGO_WPA3_SAE, DOT11_CIPHER_ALGO_CCMP), Some(Sec::Wpa3));
        assert_eq!(Sec::of(true, DOT11_AUTH_ALGO_RSNA, DOT11_CIPHER_ALGO_CCMP), None);
    }

    #[test]
    fn keys_are_checked_before_windows_sees_them() {
        let w = Sec::Wpa2 { tkip: false };
        assert_eq!(w.key_type("1234567"), Err("Password: 8 to 63 characters".into()));
        assert_eq!(w.key_type("12345678"), Ok("passPhrase"));
        assert_eq!(w.key_type(&"a".repeat(64)), Ok("networkKey"));
        assert!(w.key_type(&"z".repeat(64)).is_err());
        assert!(Sec::Wpa3.key_type(&"a".repeat(64)).is_err());
        assert_eq!(Sec::Wep { shared: false }.key_type("abcde"), Ok("networkKey"));
        assert!(Sec::Wep { shared: false }.key_type("abcdef").is_err());
        assert_eq!(Sec::Open.key_type(""), Ok(""));
    }

    #[test]
    fn profile_escapes_text_and_carries_the_ssid_as_hex() {
        let x = profile_xml("a<b", b"a<b", Sec::Wpa2 { tkip: false }, "p&ss</keyMaterial>", "passPhrase", true);
        assert!(x.contains("<hex>613C62</hex><name>a&lt;b</name>"));
        assert!(x.contains("<keyMaterial>p&amp;ss&lt;/keyMaterial&gt;</keyMaterial>"));
        assert!(x.contains("<nonBroadcast>true</nonBroadcast>") && x.contains("<authentication>WPA2PSK</authentication><encryption>AES</encryption>"));
        let o = profile_xml("open", b"open", Sec::Open, "", "", false);
        assert!(!o.contains("sharedKey") && !o.contains("nonBroadcast") && o.contains("<encryption>none</encryption>"));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore]
    fn print_wifi() {
        println!("radio: {:?}", radio());
        for n in list() {
            println!("{} q={} connected={} profile={:?} sec={:?} eap={:?}", n.ssid, n.quality, n.connected, n.profile, n.sec, n.eap);
        }
    }
}
