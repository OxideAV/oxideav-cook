//! Cook codec-specific extradata parser (§4 of the trace doc).
//!
//! Extradata blob shapes:
//!
//! | size | mode                     | layout                        |
//! |------|--------------------------|-------------------------------|
//! |  8 B | MONO / STEREO            | 4 B cookversion + 2 B spf + 2 B subbands |
//! | 16 B | JOINT_STEREO             | 8 B above + 4 B unused + 2 B js_subband_start + 2 B js_vlc_bits |
//! | 80 B | MC_COOK (5.1 / 7.1 / …)  | 4 × 20 B sub-blob (16 B blob + 4 B channel_mask) |
//!
//! All multi-byte values are big-endian.
//!
//! `samples_per_frame` is the **total** sample count summed over all
//! channels of a subpacket — for STEREO/JOINT_STEREO with 2048 spf, the
//! per-channel iMDCT length is 1024.

use oxideav_core::{Error, Result};

use crate::tables::{MAX_JS_SUBBAND_START, MAX_SUBBANDS, MAX_TOTAL_SUBBANDS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookMode {
    Mono,         // 0x01000001
    Stereo,       // 0x01000002
    JointStereo,  // 0x01000003
    MultiChannel, // 0x02000000
}

impl CookMode {
    fn from_version(v: u32) -> Result<Self> {
        match v {
            0x0100_0001 => Ok(CookMode::Mono),
            0x0100_0002 => Ok(CookMode::Stereo),
            0x0100_0003 => Ok(CookMode::JointStereo),
            0x0200_0000 => Ok(CookMode::MultiChannel),
            other => Err(Error::invalid(format!(
                "cook: unknown cookversion 0x{other:08x}"
            ))),
        }
    }

    /// Number of channels this mode emits per subpacket.
    pub fn nb_channels(self) -> usize {
        match self {
            CookMode::Mono => 1,
            CookMode::Stereo | CookMode::JointStereo => 2,
            // MC_COOK is per-subblob; use the channel_mask popcount instead.
            CookMode::MultiChannel => 0,
        }
    }

    pub fn is_joint_stereo(self) -> bool {
        matches!(self, CookMode::JointStereo)
    }
}

/// One internal subpacket's parameters — corresponds to one of the
/// 8 / 16 / 20-byte sub-blobs documented in §4.
#[derive(Debug, Clone)]
pub struct SubpacketParams {
    pub mode: CookMode,
    pub samples_per_frame: u32,
    pub subbands: u32,
    pub js_subband_start: u32,
    pub js_vlc_bits: u32,
    /// Microsoft channel-mask bitmap (MC_COOK only). For single-stream
    /// modes this is 0 (mask not stored in the wire format).
    pub channel_mask: u32,
    /// Channels emitted by this subpacket: 1 for MONO and MC_COOK mono
    /// sub-blobs, 2 for STEREO / JOINT_STEREO and MC_COOK paired
    /// sub-blobs.
    pub nb_channels: usize,
}

impl SubpacketParams {
    /// Per-channel iMDCT length. `samples_per_frame` is the total
    /// over both channels for stereo modes, so it's halved when 2-ch.
    pub fn samples_per_channel(&self) -> usize {
        (self.samples_per_frame as usize) / self.nb_channels.max(1)
    }

    pub fn validate(&self) -> Result<()> {
        if (self.js_subband_start as usize) > MAX_JS_SUBBAND_START {
            return Err(Error::invalid("cook: js_subband_start >= 51"));
        }
        if self.subbands == 0 || (self.subbands as usize) > MAX_SUBBANDS {
            return Err(Error::invalid("cook: subbands out of [1..50]"));
        }
        let total = self.subbands as usize + self.js_subband_start as usize;
        if total > MAX_TOTAL_SUBBANDS {
            return Err(Error::invalid("cook: total_subbands > 53"));
        }
        if self.mode.is_joint_stereo() {
            // 2*joint_stereo <= js_vlc_bits <= 6 → 2..=6
            if !(2..=6).contains(&self.js_vlc_bits) {
                return Err(Error::invalid(
                    "cook: js_vlc_bits out of [2..6] for joint stereo",
                ));
            }
        }
        let spc = self.samples_per_channel();
        if !matches!(spc, 256 | 512 | 1024) {
            return Err(Error::invalid(format!(
                "cook: samples_per_channel = {spc} not in {{256, 512, 1024}}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CookExtradata {
    pub subpackets: Vec<SubpacketParams>,
}

impl CookExtradata {
    pub fn parse(blob: &[u8]) -> Result<Self> {
        if blob.len() < 8 {
            return Err(Error::invalid(format!(
                "cook: extradata too short ({} B; need 8/16/80)",
                blob.len()
            )));
        }
        // Multi-channel: the first cookversion is 0x02000000 and the blob
        // is a chained sequence of 20-byte sub-blobs.
        let v0 = read_be_u32(blob, 0);
        if v0 == 0x0200_0000 {
            if blob.len() % 20 != 0 {
                return Err(Error::invalid(format!(
                    "cook MC: extradata len {} not multiple of 20",
                    blob.len()
                )));
            }
            let n = blob.len() / 20;
            let mut subpackets = Vec::with_capacity(n);
            for i in 0..n {
                let off = i * 20;
                let s = parse_sub_blob_16(&blob[off..off + 16], CookMode::MultiChannel)?;
                let mask = read_be_u32(blob, off + 16);
                let chans = mask.count_ones() as usize;
                let mut sp = s;
                sp.channel_mask = mask;
                sp.nb_channels = chans.max(1);
                sp.validate()?;
                subpackets.push(sp);
            }
            return Ok(Self { subpackets });
        }
        // Single-stream (MONO / STEREO / JOINT_STEREO).
        let mode = CookMode::from_version(v0)?;
        let needed = match mode {
            CookMode::Mono | CookMode::Stereo => 8,
            CookMode::JointStereo => 16,
            CookMode::MultiChannel => unreachable!(),
        };
        if blob.len() < needed {
            return Err(Error::invalid(format!(
                "cook: extradata too short for {mode:?} (got {} need {})",
                blob.len(),
                needed
            )));
        }
        let mut sp = parse_sub_blob_16(&blob[..needed.min(16)], mode)?;
        sp.nb_channels = mode.nb_channels();
        sp.validate()?;
        Ok(Self {
            subpackets: vec![sp],
        })
    }

    /// Total channels across all subpackets.
    pub fn total_channels(&self) -> usize {
        self.subpackets.iter().map(|s| s.nb_channels).sum()
    }
}

fn parse_sub_blob_16(buf: &[u8], default_mode: CookMode) -> Result<SubpacketParams> {
    let v = read_be_u32(buf, 0);
    let mode = if matches!(default_mode, CookMode::MultiChannel) {
        // MC sub-blobs are decoded as if they were single-stream blobs;
        // the cookversion bits have a different meaning per sub-blob,
        // but we use the standard layout the trace doc documents.
        // §4.2 example: spf=2048+joint, 1024+mono, 1024+mono(1sb), 2048+joint.
        // Determine paired vs mono by `nb_channels` after channel_mask is
        // read by the caller.
        // For now treat as JOINT_STEREO if js_vlc_bits!=0 (so the
        // joint-stereo decode path is exercised) else MONO.
        if buf.len() >= 16 {
            let js_vlc_bits = read_be_u16(buf, 14);
            if js_vlc_bits >= 2 {
                CookMode::JointStereo
            } else {
                CookMode::Mono
            }
        } else {
            CookMode::Mono
        }
    } else {
        CookMode::from_version(v)?
    };
    let samples_per_frame = read_be_u16(buf, 4) as u32;
    let subbands = read_be_u16(buf, 6) as u32;
    let (js_subband_start, js_vlc_bits) = if buf.len() >= 16 {
        (read_be_u16(buf, 12) as u32, read_be_u16(buf, 14) as u32)
    } else {
        (0, 0)
    };
    Ok(SubpacketParams {
        mode,
        samples_per_frame,
        subbands,
        js_subband_start,
        js_vlc_bits,
        channel_mask: 0,
        nb_channels: 0, // filled in by caller
    })
}

fn read_be_u16(buf: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([buf[off], buf[off + 1]])
}

fn read_be_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mono_8b() {
        // cookversion 0x01000001 + spf 1024 + subbands 25
        let blob = [0x01, 0x00, 0x00, 0x01, 0x04, 0x00, 0x00, 0x19];
        let xd = CookExtradata::parse(&blob).unwrap();
        assert_eq!(xd.subpackets.len(), 1);
        let s = &xd.subpackets[0];
        assert_eq!(s.mode, CookMode::Mono);
        assert_eq!(s.samples_per_frame, 1024);
        assert_eq!(s.subbands, 25);
        assert_eq!(s.nb_channels, 1);
        assert_eq!(s.samples_per_channel(), 1024);
    }

    #[test]
    fn parse_stereo_8b_gg_rm() {
        // gg.rm: blob = 01 00 00 02 08 00 00 25 (STEREO, spf=2048, sb=37)
        let blob = [0x01, 0x00, 0x00, 0x02, 0x08, 0x00, 0x00, 0x25];
        let xd = CookExtradata::parse(&blob).unwrap();
        let s = &xd.subpackets[0];
        assert_eq!(s.mode, CookMode::Stereo);
        assert_eq!(s.samples_per_frame, 2048);
        assert_eq!(s.subbands, 0x25);
        assert_eq!(s.nb_channels, 2);
        assert_eq!(s.samples_per_channel(), 1024);
    }

    #[test]
    fn parse_joint_stereo_16b_fun32() {
        // FUN_RM_32.rm: 01 00 00 03 08 00 00 20 00 00 00 00 00 02 00 04
        let blob = [
            0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
            0x00, 0x04,
        ];
        let xd = CookExtradata::parse(&blob).unwrap();
        let s = &xd.subpackets[0];
        assert_eq!(s.mode, CookMode::JointStereo);
        assert_eq!(s.samples_per_frame, 2048);
        assert_eq!(s.subbands, 0x20);
        assert_eq!(s.js_subband_start, 2);
        assert_eq!(s.js_vlc_bits, 4);
        assert_eq!(s.nb_channels, 2);
    }

    #[test]
    fn parse_mc_cook_80b() {
        // Surround_6ch.rma per §4.2:
        // sub-blob 0: spf=2048, sb=37, js_sb=2, js_vlc=4, mask=0x03 → L+R
        // sub-blob 1: spf=1024, sb=47, js_sb=0, js_vlc=0, mask=0x04 → C
        // sub-blob 2: spf=1024, sb=1,  js_sb=0, js_vlc=0, mask=0x08 → LFE
        // sub-blob 3: spf=2048, sb=32, js_sb=2, js_vlc=4, mask=0x30 → Ls+Rs
        let mut blob = Vec::with_capacity(80);
        // sub-blob 0
        blob.extend_from_slice(&[0x02, 0x00, 0x00, 0x00]); // ver
        blob.extend_from_slice(&2048u16.to_be_bytes());
        blob.extend_from_slice(&37u16.to_be_bytes());
        blob.extend_from_slice(&[0; 4]); // unused
        blob.extend_from_slice(&2u16.to_be_bytes()); // js_sb
        blob.extend_from_slice(&4u16.to_be_bytes()); // js_vlc
        blob.extend_from_slice(&0x0000_0003u32.to_be_bytes()); // mask
                                                               // sub-blob 1 (mono C)
        blob.extend_from_slice(&[0x02, 0x00, 0x00, 0x00]);
        blob.extend_from_slice(&1024u16.to_be_bytes());
        blob.extend_from_slice(&47u16.to_be_bytes());
        blob.extend_from_slice(&[0; 4]);
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.extend_from_slice(&0x0000_0004u32.to_be_bytes());
        // sub-blob 2 (mono LFE)
        blob.extend_from_slice(&[0x02, 0x00, 0x00, 0x00]);
        blob.extend_from_slice(&1024u16.to_be_bytes());
        blob.extend_from_slice(&1u16.to_be_bytes());
        blob.extend_from_slice(&[0; 4]);
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.extend_from_slice(&0x0000_0008u32.to_be_bytes());
        // sub-blob 3 (joint Ls/Rs)
        blob.extend_from_slice(&[0x02, 0x00, 0x00, 0x00]);
        blob.extend_from_slice(&2048u16.to_be_bytes());
        blob.extend_from_slice(&32u16.to_be_bytes());
        blob.extend_from_slice(&[0; 4]);
        blob.extend_from_slice(&2u16.to_be_bytes());
        blob.extend_from_slice(&4u16.to_be_bytes());
        blob.extend_from_slice(&0x0000_0030u32.to_be_bytes());
        assert_eq!(blob.len(), 80);

        let xd = CookExtradata::parse(&blob).unwrap();
        assert_eq!(xd.subpackets.len(), 4);
        assert_eq!(xd.total_channels(), 6);
        assert_eq!(xd.subpackets[0].channel_mask, 0x03);
        assert_eq!(xd.subpackets[0].nb_channels, 2);
        assert_eq!(xd.subpackets[1].channel_mask, 0x04);
        assert_eq!(xd.subpackets[1].nb_channels, 1);
        assert_eq!(xd.subpackets[3].channel_mask, 0x30);
        assert_eq!(xd.subpackets[3].nb_channels, 2);
    }

    #[test]
    fn rejects_bad_subbands() {
        // subbands = 0
        let blob = [0x01, 0x00, 0x00, 0x02, 0x08, 0x00, 0x00, 0x00];
        assert!(CookExtradata::parse(&blob).is_err());
    }
}
