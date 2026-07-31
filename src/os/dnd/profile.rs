//! Pure parsing/rewriting of the Windows quiet-hours CloudStore blob.
//!
//! No registry access happens here, which makes the fiddly byte-twiddling
//! unit-testable against a captured real-world blob (see the tests below).
//!
//! ## Blob layout (reverse engineered - see `super` for the warning)
//!
//! The value is a length-prefixed, protobuf-ish serialisation. We only care
//! about one field: a counted UTF-16LE string naming the active profile, e.g.
//! `Microsoft.QuietHoursProfile.Unrestricted`. From a real Windows 11 26200
//! blob (116 bytes):
//!
//! ```text
//! offset 0x12: 5e            <- container length, in bytes
//! offset 0x1c: 28            <- profile name length, in UTF-16 code units (40)
//! offset 0x1d: 4d 00 69 00.. <- "Microsoft.QuietHoursProfile.Unrestricted"
//! ```
//!
//! The two profiles we care most about are the same length:
//!
//! ```text
//! Microsoft.QuietHoursProfile.Unrestricted  (40 chars)  = DND off
//! Microsoft.QuietHoursProfile.PriorityOnly  (40 chars)  = DND on, priority allowed
//! Microsoft.QuietHoursProfile.AlarmsOnly    (38 chars)  = DND on, alarms only
//! ```
//!
//! Swapping between the first two is therefore a pure in-place substitution:
//! no length field anywhere in the blob changes, so we cannot corrupt a
//! structure we only partially understand. That is the only rewrite this module
//! performs, and it is why the app offers "on/off" rather than a choice of
//! profile.
//!
//! Rewriting to `AlarmsOnly` was implemented and tested against a live system:
//! patching the two visible length bytes produced a blob that Windows accepted
//! but *misread* (it reported priority-only). Since the surrounding container
//! format is not fully understood, any length-changing edit is refused here
//! rather than guessed at.

/// DND off.
pub const UNRESTRICTED: &str = "Microsoft.QuietHoursProfile.Unrestricted";
/// DND on, with the user's priority apps and contacts still allowed through.
/// This is what the Windows UI calls "Do not disturb".
pub const PRIORITY_ONLY: &str = "Microsoft.QuietHoursProfile.PriorityOnly";

const PREFIX: &str = "Microsoft.QuietHoursProfile.";

#[derive(Debug, PartialEq, Eq)]
pub enum ProfileError {
    /// No profile name found - the blob is not in the shape we expect.
    NotFound,
    /// The replacement name is a different length, which would require editing
    /// length fields in a container format we do not fully understand.
    /// Refused deliberately - see the module docs.
    LengthChangeRefused,
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// Locate the profile name: returns `(byte offset of the string, char count)`.
pub fn locate(blob: &[u8]) -> Option<(usize, usize)> {
    let needle = utf16le(PREFIX);
    let pos = blob.windows(needle.len()).position(|w| w == needle)?;

    let count = *blob.get(pos.checked_sub(1)?)? as usize;

    if pos + count * 2 > blob.len() {
        return None;
    }
    Some((pos, count))
}

/// The profile name currently stored in `blob`.
pub fn read(blob: &[u8]) -> Option<String> {
    let (pos, count) = locate(blob)?;
    let units: Vec<u16> = (0..count)
        .map(|i| u16::from_le_bytes([blob[pos + i * 2], blob[pos + i * 2 + 1]]))
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// Offset of the "last modified" field: tag `2a 06` at byte 8, then a five-byte
/// LEB128 Unix timestamp.
const STAMP_TAG: usize = 8;
const STAMP_AT: usize = 10;
const STAMP_LEN: usize = 5;

/// Overwrite the blob's timestamp with `unix_seconds`.
///
/// This matters more than it looks. CloudStore uses the timestamp to decide
/// which copy of a setting wins, and there are two containers holding this
/// value. Writing the profile name while leaving an old timestamp in place left
/// the two disagreeing, and Explorer went on trusting its cached copy - so
/// notifications were genuinely muted while the taskbar indicator never
/// updated, even for changes the user made themselves.
///
/// Five bytes hold any timestamp until the year 2106, so the blob never changes
/// length and the same-length guarantee above still holds.
fn stamp(out: &mut [u8], unix_seconds: u64) {
    if out.len() < STAMP_AT + STAMP_LEN || out[STAMP_TAG] != 0x2a || out[STAMP_TAG + 1] != 0x06 {
        return;
    }
    let mut v = unix_seconds;
    for i in 0..STAMP_LEN {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        out[STAMP_AT + i] = if i < STAMP_LEN - 1 { byte | 0x80 } else { byte };
    }
}

/// Read the blob's timestamp, if it is where we expect it.
///
/// Diagnostic counterpart to [`stamp`]; used by the tests that pin the encoding
/// down, and useful when investigating a state that will not change.
#[allow(dead_code)]
pub fn read_stamp(blob: &[u8]) -> Option<u64> {
    if blob.len() < STAMP_AT + STAMP_LEN || blob[STAMP_TAG] != 0x2a || blob[STAMP_TAG + 1] != 0x06 {
        return None;
    }
    let mut v = 0u64;
    for i in 0..STAMP_LEN {
        v |= ((blob[STAMP_AT + i] & 0x7F) as u64) << (7 * i);
    }
    Some(v)
}

/// Return a copy of `blob` with the active profile replaced by `name` and the
/// timestamp set to `unix_seconds`.
///
/// Only same-length replacements are performed; anything else is refused so a
/// partially-understood container format can never be corrupted.
pub fn write(blob: &[u8], name: &str, unix_seconds: u64) -> Result<Vec<u8>, ProfileError> {
    let (pos, old_count) = locate(blob).ok_or(ProfileError::NotFound)?;
    let new_count = name.encode_utf16().count();
    if new_count != old_count {
        return Err(ProfileError::LengthChangeRefused);
    }
    let mut out = blob.to_vec();
    out[pos..pos + new_count * 2].copy_from_slice(&utf16le(name));
    stamp(&mut out, unix_seconds);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A verbatim capture of `windows.data.donotdisturb.quiethourssettings\Data`
    /// from Windows 11 Pro 26200, with DND off.
    const REAL_BLOB: [u8; 116] = [
        0x43, 0x42, 0x01, 0x00, 0x0a, 0x02, 0x01, 0x00, 0x2a, 0x06, 0x8c, 0x95, 0xfd, 0xc5, 0x06,
        0x2a, 0x2b, 0x0e, 0x5e, 0x43, 0x42, 0x01, 0x00, 0xc2, 0x0a, 0x01, 0xd2, 0x14, 0x28, 0x4d,
        0x00, 0x69, 0x00, 0x63, 0x00, 0x72, 0x00, 0x6f, 0x00, 0x73, 0x00, 0x6f, 0x00, 0x66, 0x00,
        0x74, 0x00, 0x2e, 0x00, 0x51, 0x00, 0x75, 0x00, 0x69, 0x00, 0x65, 0x00, 0x74, 0x00, 0x48,
        0x00, 0x6f, 0x00, 0x75, 0x00, 0x72, 0x00, 0x73, 0x00, 0x50, 0x00, 0x72, 0x00, 0x6f, 0x00,
        0x66, 0x00, 0x69, 0x00, 0x6c, 0x00, 0x65, 0x00, 0x2e, 0x00, 0x55, 0x00, 0x6e, 0x00, 0x72,
        0x00, 0x65, 0x00, 0x73, 0x00, 0x74, 0x00, 0x72, 0x00, 0x69, 0x00, 0x63, 0x00, 0x74, 0x00,
        0x65, 0x00, 0x64, 0x00, 0xca, 0x28, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    const PRIORITY: &str = "Microsoft.QuietHoursProfile.PriorityOnly";
    const ALARMS: &str = "Microsoft.QuietHoursProfile.AlarmsOnly";

    #[test]
    fn reads_the_real_blob() {
        assert_eq!(locate(&REAL_BLOB), Some((0x1d, 40)));
        assert_eq!(read(&REAL_BLOB).as_deref(), Some(UNRESTRICTED));
    }

    /// The timestamp already in `REAL_BLOB`.
    const ORIGINAL_STAMP: u64 = 1_757_366_924;

    #[test]
    fn same_length_swap_touches_only_the_name_and_stamp() {
        let out = write(&REAL_BLOB, PRIORITY, ORIGINAL_STAMP).unwrap();
        assert_eq!(out.len(), REAL_BLOB.len(), "length must not change");
        assert_eq!(read(&out).as_deref(), Some(PRIORITY));
        assert_eq!(out, {
            let mut expect = REAL_BLOB.to_vec();
            let (pos, count) = locate(&REAL_BLOB).unwrap();
            expect[pos..pos + count * 2].copy_from_slice(&utf16le(PRIORITY));
            expect
        });
    }

    #[test]
    fn round_trip_returns_original_bytes() {
        let on = write(&REAL_BLOB, PRIORITY, ORIGINAL_STAMP).unwrap();
        let off = write(&on, UNRESTRICTED, ORIGINAL_STAMP).unwrap();
        assert_eq!(off, REAL_BLOB, "toggling back must be byte-identical");
    }

    /// Writing the name while leaving an old timestamp behind left the two
    /// CloudStore containers disagreeing, and Explorer went on trusting its
    /// cached copy - notifications were muted but the taskbar indicator stopped
    /// updating, even for changes the user made themselves.
    #[test]
    fn the_timestamp_is_refreshed_without_changing_the_length() {
        assert_eq!(read_stamp(&REAL_BLOB), Some(ORIGINAL_STAMP));

        let now = 1_785_520_746;
        let out = write(&REAL_BLOB, PRIORITY, now).unwrap();
        assert_eq!(out.len(), REAL_BLOB.len(), "five bytes always suffice");
        assert_eq!(read_stamp(&out), Some(now));
        assert_eq!(read(&out).as_deref(), Some(PRIORITY));

        // Every timestamp up to 2106 fits the same five bytes.
        for t in [0u64, 1, ORIGINAL_STAMP, now, (1u64 << 35) - 1] {
            let b = write(&REAL_BLOB, PRIORITY, t).unwrap();
            assert_eq!(b.len(), REAL_BLOB.len(), "stamp {t} changed the length");
            assert_eq!(read_stamp(&b), Some(t));
        }
    }

    #[test]
    fn refuses_to_resize_the_blob() {
        assert_eq!(ALARMS.len(), 38);
        assert_eq!(
            write(&REAL_BLOB, ALARMS, ORIGINAL_STAMP),
            Err(ProfileError::LengthChangeRefused)
        );
    }

    #[test]
    fn the_two_profiles_we_use_are_interchangeable_in_place() {
        assert_eq!(UNRESTRICTED.len(), PRIORITY.len());
    }

    #[test]
    fn rejects_a_truncated_blob_instead_of_panicking() {
        for cut in 0..REAL_BLOB.len() {
            let truncated = &REAL_BLOB[..cut];

            let _ = read(truncated);
            let _ = read_stamp(truncated);
            let _ = write(truncated, PRIORITY, ORIGINAL_STAMP);
            let _ = write(truncated, ALARMS, ORIGINAL_STAMP);
        }
    }
}
