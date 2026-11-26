use std::net::{Ipv4Addr, Ipv6Addr};
use windows_sys::Win32::{
    Foundation::*,
    NetworkManagement::IpHelper::*,
    Networking::WinSock::{SOCKADDR, SOCKADDR_IN6, SOCKET_ADDRESS},
};

const IF_ETHERNET: u32 = 6;
const IF_WIFI: u32 = 71;
const OPER_UP: i32 = 1;

pub struct Adapter {
    pub name: String,
    pub desc: String,
    pub wifi: bool,
    pub up: bool,
    pub speed: u64,
    pub ipv4: Vec<(Ipv4Addr, u8)>,
    pub ipv6: Vec<Ipv6Addr>,
    pub gateway: Vec<String>,
    pub dns: Vec<String>,
    pub mac: String,
    pub dhcp: bool,
    pub internet: Option<bool>,
}

pub fn speed_text(bits_per_sec: u64) -> Option<String> {
    match bits_per_sec / 1_000_000 {
        0 => None,
        m if m >= 1000 => Some(format!("{} Gbps", m as f64 / 1000.0)),
        m => Some(format!("{} Mbps", m)),
    }
}

fn apipa(ip: &Ipv4Addr) -> bool {
    ip.octets()[..2] == [169, 254]
}

impl Adapter {
    pub fn status(&self) -> String {
        if !self.up {
            return "Not connected".into();
        }
        match (self.internet, self.ipv4.first()) {
            (Some(true), _) => "Internet".into(),
            (_, Some((ip, _))) if apipa(ip) && self.ipv6.is_empty() => "No address (DHCP)".into(),
            (Some(false), _) => "No internet".into(),
            _ => "Connected".into(),
        }
    }

    pub fn summary(&self) -> String {
        let ip = self.ipv4.iter().find(|(ip, _)| !apipa(ip)).map(|(ip, _)| ip.to_string()).or_else(|| self.ipv6.first().map(|ip| ip.to_string()));
        let kind = if self.wifi { "Wi-Fi" } else { "Ethernet" };
        [Some(kind.to_string()), ip, if self.up { speed_text(self.speed) } else { None }].into_iter().flatten().collect::<Vec<_>>().join("  ·  ")
    }

    pub fn details(&self) -> Vec<(String, String)> {
        let join = |v: Vec<String>| if v.is_empty() { "—".to_string() } else { v.join(", ") };
        let mut out = vec![("Description".to_string(), self.desc.clone())];
        if self.up {
            out.push(("IPv4".into(), join(self.ipv4.iter().map(|(ip, p)| format!("{ip}/{p}")).collect())));
            if !self.ipv6.is_empty() {
                out.push(("IPv6".into(), join(self.ipv6.iter().map(|ip| ip.to_string()).collect())));
            }
            out.push(("Gateway".into(), join(self.gateway.clone())));
            out.push(("DNS".into(), join(self.dns.clone())));
        }
        out.push(("IP assignment".into(), if self.dhcp { "Automatic (DHCP)".into() } else { "Manual".into() }));
        out.push(("MAC".into(), self.mac.clone()));
        out
    }
}

fn pwstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

fn sockaddr(a: &SOCKET_ADDRESS) -> Option<std::net::IpAddr> {
    let sa = a.lpSockaddr as *const SOCKADDR;
    if sa.is_null() {
        return None;
    }
    match unsafe { (*sa).sa_family } {
        2 => {
            let d = unsafe { (*sa).sa_data };
            Some(Ipv4Addr::new(d[2] as u8, d[3] as u8, d[4] as u8, d[5] as u8).into())
        }
        23 if a.iSockaddrLength as usize >= size_of::<SOCKADDR_IN6>() => Some(Ipv6Addr::from(unsafe { (*(sa as *const SOCKADDR_IN6)).sin6_addr.u.Byte }).into()),
        _ => None,
    }
}

macro_rules! walk {
    ($first:expr) => {{
        let mut out = vec![];
        let mut p = $first;
        while !p.is_null() {
            let x = unsafe { &*p };
            out.push(x);
            p = x.Next;
        }
        out
    }};
}

fn hardware(a: &IP_ADAPTER_ADDRESSES_LH) -> bool {
    let mut row: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
    row.InterfaceLuid = a.Luid;
    unsafe { GetIfEntry2(&mut row) == ERROR_SUCCESS && row.InterfaceAndOperStatusFlags._bitfield & 1 != 0 }
}

fn internet() -> Vec<(windows::core::GUID, bool)> {
    use windows::Win32::{Networking::NetworkListManager::*, System::Com::*};
    let mut out = vec![];
    unsafe {
        let Ok(nlm) = CoCreateInstance::<_, INetworkListManager>(&NetworkListManager, None, CLSCTX_ALL) else { return out };
        let Ok(conns) = nlm.GetNetworkConnections() else { return out };
        loop {
            let mut one = [None];
            let mut n = 0u32;
            if conns.Next(&mut one, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            let Some(c) = one[0].take() else { break };
            if let (Ok(id), Ok(cn)) = (c.GetAdapterId(), c.GetConnectivity()) {
                out.push((id, cn.0 & (NLM_CONNECTIVITY_IPV4_INTERNET.0 | NLM_CONNECTIVITY_IPV6_INTERNET.0) != 0));
            }
        }
    }
    out
}

fn guid_of(name: &str) -> Option<windows::core::GUID> {
    let h = name.trim_matches(|c| c == '{' || c == '}').replace('-', "");
    u128::from_str_radix(&h, 16).ok().filter(|_| h.len() == 32).map(windows::core::GUID::from_u128)
}

pub fn adapters() -> Vec<Adapter> {
    let mut n = 16 * 1024u32;
    let mut buf: Vec<u64> = vec![0; n as usize / 8 + 1];
    for _ in 0..3 {
        let rc = unsafe { GetAdaptersAddresses(0, GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_INCLUDE_GATEWAYS, std::ptr::null(), buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH, &mut n) };
        if rc == ERROR_BUFFER_OVERFLOW {
            buf = vec![0; n as usize / 8 + 1];
            continue;
        }
        if rc != ERROR_SUCCESS {
            break;
        }
        let net = internet();
        let mut out: Vec<Adapter> = walk!(buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH)
            .into_iter()
            .filter(|a| (a.IfType == IF_ETHERNET || a.IfType == IF_WIFI) && hardware(a))
            .map(|a| {
                let mut ipv4 = vec![];
                let mut ipv6 = vec![];
                for u in walk!(a.FirstUnicastAddress) {
                    match sockaddr(&u.Address) {
                        Some(std::net::IpAddr::V4(ip)) => ipv4.push((ip, u.OnLinkPrefixLength)),
                        Some(std::net::IpAddr::V6(ip)) if ip.segments()[0] & 0xffc0 != 0xfe80 => ipv6.push(ip),
                        _ => {}
                    }
                }
                let name = unsafe { std::ffi::CStr::from_ptr(a.AdapterName as _) }.to_string_lossy().into_owned();
                let id = guid_of(&name);
                let mac = a.PhysicalAddress[..(a.PhysicalAddressLength as usize).min(8)].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
                Adapter {
                    name: pwstr(a.FriendlyName),
                    desc: pwstr(a.Description),
                    wifi: a.IfType == IF_WIFI,
                    up: a.OperStatus == OPER_UP,
                    speed: a.TransmitLinkSpeed,
                    ipv4,
                    ipv6,
                    gateway: walk!(a.FirstGatewayAddress).into_iter().filter_map(|g| sockaddr(&g.Address)).map(|ip| ip.to_string()).collect(),
                    dns: walk!(a.FirstDnsServerAddress).into_iter().filter_map(|d| sockaddr(&d.Address)).map(|ip| ip.to_string()).fold(vec![], |mut v, ip| {
                        if !v.contains(&ip) {
                            v.push(ip);
                        }
                        v
                    }),
                    mac,
                    dhcp: unsafe { a.Anonymous2.Flags } & IP_ADAPTER_DHCP_ENABLED != 0,
                    internet: id.and_then(|id| net.iter().find(|(g, _)| *g == id).map(|&(_, on)| on)),
                }
            })
            .collect();
        out.sort_by_key(|a| !a.up);
        return out;
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eth(up: bool, ip: [u8; 4], internet: Option<bool>) -> Adapter {
        Adapter { name: "Ethernet".into(), desc: String::new(), wifi: false, up, speed: 1_000_000_000, ipv4: vec![(Ipv4Addr::from(ip), 24)], ipv6: vec![], gateway: vec![], dns: vec![], mac: String::new(), dhcp: true, internet }
    }

    #[test]
    fn link_speed_reads_in_mbps_or_gbps() {
        assert_eq!(speed_text(0), None);
        assert_eq!(speed_text(866_000_000).as_deref(), Some("866 Mbps"));
        assert_eq!(speed_text(2_500_000_000).as_deref(), Some("2.5 Gbps"));
    }

    #[test]
    fn status_tells_internet_dhcp_failure_and_unplugged_apart() {
        assert_eq!(eth(false, [192, 168, 1, 5], None).status(), "Not connected");
        assert_eq!(eth(true, [192, 168, 1, 5], Some(true)).status(), "Internet");
        assert_eq!(eth(true, [169, 254, 3, 4], Some(false)).status(), "No address (DHCP)");
        assert_eq!(eth(true, [192, 168, 1, 5], Some(false)).status(), "No internet");
        assert_eq!(eth(true, [192, 168, 1, 5], None).status(), "Connected");
        assert_eq!(eth(true, [192, 168, 1, 5], None).summary(), "Ethernet  ·  192.168.1.5  ·  1 Gbps");
        assert_eq!(eth(false, [169, 254, 3, 4], None).summary(), "Ethernet");
    }

    #[test]
    fn adapter_guids_parse() {
        assert_eq!(guid_of("{12345678-9ABC-DEF0-1234-56789ABCDEF0}"), Some(windows::core::GUID::from_u128(0x12345678_9abc_def0_1234_56789abcdef0)));
        assert_eq!(guid_of("junk"), None);
    }

    #[test]
    #[ignore]
    fn print_adapters() {
        unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED).ok().ok() };
        for a in adapters() {
            println!("{} | {} | {} | {:?}", a.name, a.status(), a.summary(), a.details());
        }
    }
}
