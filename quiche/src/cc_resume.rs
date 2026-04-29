// Copyright (C) 2018-2019, Cloudflare, Inc.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
//     * Redistributions of source code must retain the above copyright notice,
//       this list of conditions and the following disclaimer.
//
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS
// IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO,
// THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
// PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
// CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
// EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
// PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
// PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
// LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
// NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! Server congestion resume: encoded `cc_state` payload and HMAC over the
//! extension frames (`CC_INDICATION` / `CC_RESUME`).

use ring::digest;
use ring::hmac;
use octets;

use crate::recovery::CongestionControlAlgorithm;
use crate::Error;
use crate::Result;

/// Magic header for `cc_state` blobs.
const CC_STATE_MAGIC: &[u8; 4] = b"QCCR";

/// Wire format version embedded in `cc_state`.
pub const CC_STATE_VERSION: u8 = 1;

/// Default HMAC key material when [`crate::Config::set_cc_resume_hmac_key`] is
/// not used. **Deployable services should set an explicit key** via config.
pub fn default_cc_resume_hmac_key() -> [u8; 64] {
    let d = digest::digest(
        &digest::SHA512,
        b"quiche cc resume default hmac do not use in production",
    );

    let mut k = [0u8; 64];
    k.copy_from_slice(d.as_ref());
    k
}
/// Derives the XOR mask for obfuscating the epoch from the master key.
fn epoch_mask(key: &[u8; 64]) -> u64 {
    let sk = hmac::Key::new(hmac::HMAC_SHA512, key.as_slice());
    let tag = hmac::sign(&sk, b"cc-resume-epoch");
    u64::from_be_bytes(tag.as_ref()[0..8].try_into().unwrap()) &
        octets::MAX_VAR_INT
}

/// Obfuscates the wire epoch using a reversible XOR mask.
pub fn obfuscate_epoch(epoch: u64, key: &[u8; 64]) -> u64 {
    (epoch & octets::MAX_VAR_INT) ^ epoch_mask(key)
}

/// Derives a per-epoch key from the master key and the obfuscated epoch.
pub fn derive_cc_resume_key_from_epoch(
    master_key: &[u8; 64], epoch_obf: u64,
) -> [u8; 64] {
    let sk = hmac::Key::new(hmac::HMAC_SHA512, master_key.as_slice());
    let tag = hmac::sign(&sk, &epoch_obf.to_be_bytes());

    let mut out = [0u8; 64];
    out.copy_from_slice(tag.as_ref());
    out
}

/// Parsed congestion snapshot carried in `cc_state`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCcState {
    /// Server wall-clock milliseconds since UNIX epoch when the indication was
    /// built.
    pub wall_time_ms: u64,

    /// Congestion window in bytes.
    pub cwnd: u64,

    /// Smoothed RTT in nanoseconds.
    pub rtt_ns: u64,

    /// Delivery rate in bits per second (see [`crate::recovery::Bandwidth`]).
    pub delivery_bps: u64,

    /// Congestion-control algorithm tag stored at emission time.
    pub cc_algorithm: CongestionControlAlgorithm,
}

fn cc_algo_to_tag(a: CongestionControlAlgorithm) -> u8 {
    match a {
        CongestionControlAlgorithm::Reno => 1,
        CongestionControlAlgorithm::CUBIC => 2,
        CongestionControlAlgorithm::Bbr2Gcongestion => 3,
    }
}

fn cc_algo_from_tag(t: u8) -> Result<CongestionControlAlgorithm> {
    match t {
        1 => Ok(CongestionControlAlgorithm::Reno),
        2 => Ok(CongestionControlAlgorithm::CUBIC),
        3 => Ok(CongestionControlAlgorithm::Bbr2Gcongestion),
        _ => Err(Error::InvalidFrame),
    }
}

/// Encodes [`ParsedCcState`] as `cc_state`.
pub fn encode_cc_state(s: &ParsedCcState) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 1 + 2 + 8 * 4);
    out.extend_from_slice(CC_STATE_MAGIC);
    out.push(CC_STATE_VERSION);
    out.push(0);
    out.push(cc_algo_to_tag(s.cc_algorithm));
    out.push(0);
    out.extend_from_slice(&s.wall_time_ms.to_be_bytes());
    out.extend_from_slice(&s.cwnd.to_be_bytes());
    out.extend_from_slice(&s.rtt_ns.to_be_bytes());
    out.extend_from_slice(&s.delivery_bps.to_be_bytes());
    out
}

/// Decodes a `cc_state` blob.
pub fn decode_cc_state(data: &[u8]) -> Result<ParsedCcState> {
    if data.len() < 40 {
        return Err(Error::InvalidFrame);
    }

    if &data[0..4] != CC_STATE_MAGIC.as_slice() {
        return Err(Error::InvalidFrame);
    }

    if data[4] != CC_STATE_VERSION {
        return Err(Error::InvalidFrame);
    }

    let cc_algorithm = cc_algo_from_tag(data[6])?;

    let wall_time_ms = u64::from_be_bytes(data[8..16].try_into().unwrap());
    let cwnd = u64::from_be_bytes(data[16..24].try_into().unwrap());
    let rtt_ns = u64::from_be_bytes(data[24..32].try_into().unwrap());
    let delivery_bps = u64::from_be_bytes(data[32..40].try_into().unwrap());

    Ok(ParsedCcState {
        wall_time_ms,
        cwnd,
        rtt_ns,
        delivery_bps,
        cc_algorithm,
    })
}

/// Builds the authentication tag bytes placed in the `Hash` frame field.
///
/// The tag is `HMAC-SHA512(k, epoch_be || cc_state)`.
pub fn compute_cc_resume_mac(
    key: &[u8; 64], epoch: u64, cc_state: &[u8],
) -> Vec<u8> {
    let sk = hmac::Key::new(hmac::HMAC_SHA512, key.as_slice());
    let mut ctx = hmac::Context::with_key(&sk);
    ctx.update(&epoch.to_be_bytes());
    ctx.update(cc_state);
    ctx.sign().as_ref().to_vec()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() &&
        a.iter()
            .zip(b.iter())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y)) ==
            0
}

/// Returns `true` if `tag` is a valid MAC for `epoch` and `cc_state`.
pub fn verify_cc_resume_mac(
    key: &[u8; 64], epoch: u64, cc_state: &[u8], tag: &[u8],
) -> bool {
    if tag.is_empty() {return false;}

    let expected = compute_cc_resume_mac(key, epoch, cc_state);
    if tag.len() > expected.len() {return false;}
    // Compares the expected tag (truncated to the provided tag length) 
    // with the given tag in constant time to prevent timing attacks.
    constant_time_eq(&expected.as_slice()[..tag.len()], tag)
}

/// Maximum supported hash length (bytes) for CC resume MAC.
pub const CC_RESUME_HASH_MAX_LEN: usize = 64;

/// Maximum age of `wall_time_ms` in [`ParsedCcState`] for accepting a resume
/// (server MAY ignore stale state).
pub const CC_RESUME_MAX_STATE_AGE_MS: u64 = 24 * 3600 * 1000;

/// Returns current UNIX time in milliseconds (best effort).
pub fn wall_time_ms_now() -> u64 {
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Returns `true` if the advertised wall time is not older than
/// [`CC_RESUME_MAX_STATE_AGE_MS`] relative to `now_ms`.
pub fn cc_state_not_expired(wall_time_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(wall_time_ms) <= CC_RESUME_MAX_STATE_AGE_MS
}
