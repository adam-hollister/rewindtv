//! HDHomeRun network discovery (UDP port 65001), so Plex finds RewindTV as a
//! tuner without typing in an address. Needs host networking: broadcasts
//! don't cross Docker's default bridge network.

use std::net::{IpAddr, SocketAddr};

use tokio::net::UdpSocket;

use crate::state::AppState;

pub const DISCOVERY_PORT: u16 = 65001;

const TYPE_DISCOVER_REQ: u16 = 0x0002;
const TYPE_DISCOVER_RPY: u16 = 0x0003;
const TAG_DEVICE_TYPE: u8 = 0x01;
const TAG_DEVICE_ID: u8 = 0x02;
const TAG_TUNER_COUNT: u8 = 0x10;
const TAG_LINEUP_URL: u8 = 0x27;
const TAG_BASE_URL: u8 = 0x2A;
const TAG_DEVICE_AUTH: u8 = 0x2B;
const DEVICE_TYPE_TUNER: u32 = 0x0000_0001;
const WILDCARD: u32 = 0xFFFF_FFFF;

/// HDHomeRun device ids carry a checksum that clients validate; derive a
/// valid one per profile.
pub fn device_id(profile: i64) -> u32 {
    const LOOKUP: [u8; 16] = [0xA, 0x5, 0xF, 0x6, 0x7, 0xC, 0x1, 0xB, 0x9, 0x2, 0x8, 0xD, 0x4, 0x3, 0xE, 0x0];
    let base: u32 = 0x5257_4000 | ((profile as u32 & 0xFF) << 4);
    let mut sum = 0u8;
    for (i, shift) in [28, 24, 20, 16, 12, 8, 4].into_iter().enumerate() {
        let nibble = ((base >> shift) & 0xF) as usize;
        sum ^= if i % 2 == 0 { LOOKUP[nibble] } else { nibble as u8 };
    }
    // The last nibble is XORed in directly, so choosing it equal to the
    // running checksum makes the total zero.
    base | sum as u32
}

#[cfg(test)]
pub fn valid_device_id(id: u32) -> bool {
    const LOOKUP: [u8; 16] = [0xA, 0x5, 0xF, 0x6, 0x7, 0xC, 0x1, 0xB, 0x9, 0x2, 0x8, 0xD, 0x4, 0x3, 0xE, 0x0];
    let mut sum = 0u8;
    for (i, shift) in [28, 24, 20, 16, 12, 8, 4, 0].into_iter().enumerate() {
        let nibble = ((id >> shift) & 0xF) as usize;
        sum ^= if i % 2 == 0 { LOOKUP[nibble] } else { nibble as u8 };
    }
    sum == 0
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn push_tlv(buf: &mut Vec<u8>, tag: u8, value: &[u8]) {
    buf.push(tag);
    let len = value.len();
    if len < 0x80 {
        buf.push(len as u8);
    } else {
        buf.push((len as u8 & 0x7F) | 0x80);
        buf.push((len >> 7) as u8);
    }
    buf.extend_from_slice(value);
}

fn frame(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(payload.len() + 8);
    pkt.extend_from_slice(&kind.to_be_bytes());
    pkt.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    pkt.extend_from_slice(payload);
    let crc = crc32(&pkt);
    pkt.extend_from_slice(&crc.to_le_bytes());
    pkt
}

/// Tags from a valid packet of the given type.
fn parse(pkt: &[u8], kind: u16) -> Option<Vec<(u8, Vec<u8>)>> {
    if pkt.len() < 8 || u16::from_be_bytes([pkt[0], pkt[1]]) != kind {
        return None;
    }
    let len = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
    if pkt.len() != len + 8 {
        return None;
    }
    let (body, crc) = pkt.split_at(len + 4);
    if crc32(body) != u32::from_le_bytes([crc[0], crc[1], crc[2], crc[3]]) {
        return None;
    }
    let mut tags = Vec::new();
    let mut p = &body[4..];
    while p.len() >= 2 {
        let tag = p[0];
        let (vlen, hdr) = if p[1] & 0x80 != 0 {
            (((p[1] & 0x7F) as usize) | (*p.get(2)? as usize) << 7, 3)
        } else {
            (p[1] as usize, 2)
        };
        let value = p.get(hdr..hdr + vlen)?.to_vec();
        tags.push((tag, value));
        p = &p[hdr + vlen..];
    }
    Some(tags)
}

fn be32(v: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(v.try_into().ok()?))
}

/// Should we answer this discover request?
fn wants_us(tags: &[(u8, Vec<u8>)], id: u32) -> bool {
    tags.iter().all(|(tag, v)| match *tag {
        TAG_DEVICE_TYPE => be32(v).is_some_and(|t| t == DEVICE_TYPE_TUNER || t == WILDCARD),
        TAG_DEVICE_ID => be32(v).is_some_and(|d| d == id || d == WILDCARD),
        _ => true,
    })
}

pub fn reply(id: u32, tuners: u8, base_url: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    push_tlv(&mut payload, TAG_DEVICE_TYPE, &DEVICE_TYPE_TUNER.to_be_bytes());
    push_tlv(&mut payload, TAG_DEVICE_ID, &id.to_be_bytes());
    push_tlv(&mut payload, TAG_TUNER_COUNT, &[tuners]);
    push_tlv(&mut payload, TAG_DEVICE_AUTH, b"rewindtv");
    push_tlv(&mut payload, TAG_BASE_URL, base_url.as_bytes());
    push_tlv(&mut payload, TAG_LINEUP_URL, format!("{base_url}/lineup.json").as_bytes());
    frame(TYPE_DISCOVER_RPY, &payload)
}

/// Our address as seen from `peer` (the interface the OS would route through).
async fn local_ip_for(peer: SocketAddr) -> Option<IpAddr> {
    let probe = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    probe.connect(peer).await.ok()?;
    Some(probe.local_addr().ok()?.ip())
}

/// Answer HDHomeRun discovery broadcasts until the process exits.
pub async fn serve(st: AppState) {
    let sock = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::info!("HDHomeRun discovery disabled (UDP {DISCOVERY_PORT}: {e}); add the tuner in Plex by address");
            return;
        }
    };
    let _ = sock.set_broadcast(true);
    let id = device_id(st.cfg.hdhr_profile);
    let mut buf = [0u8; 1500];
    loop {
        let Ok((n, peer)) = sock.recv_from(&mut buf).await else { continue };
        let Some(tags) = parse(&buf[..n], TYPE_DISCOVER_REQ) else { continue };
        if !wants_us(&tags, id) {
            continue;
        }
        let base = match &st.cfg.public_url {
            Some(u) => u.clone(),
            None => match local_ip_for(peer).await {
                Some(ip) => format!("http://{ip}:{}", st.cfg.port),
                None => continue,
            },
        };
        tracing::debug!("HDHomeRun discovery from {peer}, answering with {base}");
        let _ = sock.send_to(&reply(id, st.cfg.tuners, &base), peer).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ids_pass_the_checksum() {
        for p in 0..300 {
            assert!(valid_device_id(device_id(p)), "profile {p}");
        }
        assert!(!valid_device_id(device_id(1) ^ 1));
    }

    #[test]
    fn crc_matches_ethernet_crc32() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn discover_round_trip() {
        let mut req = Vec::new();
        push_tlv(&mut req, TAG_DEVICE_TYPE, &DEVICE_TYPE_TUNER.to_be_bytes());
        push_tlv(&mut req, TAG_DEVICE_ID, &WILDCARD.to_be_bytes());
        let req = frame(TYPE_DISCOVER_REQ, &req);
        let tags = parse(&req, TYPE_DISCOVER_REQ).unwrap();
        assert!(wants_us(&tags, device_id(0)));

        let mut other = Vec::new();
        push_tlv(&mut other, TAG_DEVICE_ID, &0x1234_5678u32.to_be_bytes());
        assert!(!wants_us(&parse(&frame(TYPE_DISCOVER_REQ, &other), TYPE_DISCOVER_REQ).unwrap(), device_id(0)));

        let long_url = format!("http://192.168.1.20:8420/{}", "x".repeat(150));
        let rpy = reply(device_id(0), 6, &long_url);
        let tags = parse(&rpy, TYPE_DISCOVER_RPY).unwrap();
        let get = |t: u8| tags.iter().find(|(k, _)| *k == t).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(be32(&get(TAG_DEVICE_ID)), Some(device_id(0)));
        assert_eq!(get(TAG_TUNER_COUNT), vec![6]);
        assert_eq!(String::from_utf8(get(TAG_BASE_URL)).unwrap(), long_url);
        assert_eq!(String::from_utf8(get(TAG_LINEUP_URL)).unwrap(), format!("{long_url}/lineup.json"));

        let mut corrupt = rpy.clone();
        corrupt[6] ^= 0xFF;
        assert!(parse(&corrupt, TYPE_DISCOVER_RPY).is_none());
    }
}
