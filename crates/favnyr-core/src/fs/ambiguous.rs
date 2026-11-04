//! Bounded, allocation-free recognition for explicitly ambiguous extensions.
//! Listing stays extension-only; callers supply bytes from an allowed worker read.

use super::FileKind;

pub const PROBE_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    TransportStream,
}

/// Ordinary extensions have no content probe. Unknown content stays generic.
pub fn rule_for_extension(extension: &str) -> Option<Rule> {
    extension
        .eq_ignore_ascii_case("ts")
        .then_some(Rule::TransportStream)
}

impl Rule {
    pub fn classify(self, prefix: &[u8]) -> FileKind {
        let prefix = &prefix[..prefix.len().min(PROBE_BYTES)];
        match self {
            Self::TransportStream if is_transport_stream(prefix) => FileKind::Video,
            _ => FileKind::File,
        }
    }
}

fn is_transport_stream(prefix: &[u8]) -> bool {
    const PACKET_BYTES: usize = 188;
    const PACKETS: usize = 5;
    // Accommodate timestamp prefixes and error-correction tails as well as
    // plain packets. A lone sync byte, or readable text, proves nothing.
    for stride in [188, 192, 204] {
        for offset in 0..stride {
            let end = offset + (PACKETS - 1) * stride + PACKET_BYTES;
            if end > prefix.len() {
                break;
            }
            if (0..PACKETS).all(|index| {
                let packet = &prefix[offset + index * stride..][..PACKET_BYTES];
                let control = (packet[3] >> 4) & 3;
                packet[0] == 0x47
                    && packet[1] & 0x80 == 0
                    && control != 0
                    && (control & 2 == 0 || usize::from(packet[4]) <= PACKET_BYTES - 5)
            }) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packets(stride: usize, offset: usize) -> Vec<u8> {
        let mut bytes = vec![0xff; offset + stride * 6];
        for index in 0..6 {
            let start = offset + index * stride;
            bytes[start..start + 4].copy_from_slice(&[0x47, 0x01, 0x00, 0x10 | index as u8]);
        }
        bytes
    }

    #[test]
    fn only_registered_extensions_request_a_probe_and_listing_stays_generic() {
        for extension in ["ts", "TS", "Ts"] {
            assert_eq!(rule_for_extension(extension), Some(Rule::TransportStream));
            assert_eq!(
                super::super::classify_kind(Some(extension), false),
                FileKind::File
            );
        }
        for extension in ["txt", "mp4", "png", "obj", "mts", "cts", ""] {
            assert!(rule_for_extension(extension).is_none());
        }
        assert_eq!(
            super::super::classify_kind(Some("ts"), true),
            FileKind::Folder
        );
        assert_eq!(
            super::super::classify_kind(Some("mp4"), false),
            FileKind::Video
        );
    }

    #[test]
    fn packet_headers_support_all_three_strides_and_an_initial_offset() {
        for stride in [188, 192, 204] {
            for offset in [0, 4, 17] {
                assert_eq!(
                    Rule::TransportStream.classify(&packets(stride, offset)),
                    FileKind::Video
                );
            }
        }
    }

    #[test]
    fn text_empty_short_and_inconsistent_binary_inputs_stay_generic() {
        for bytes in [
            b"export const value = 1;\n".as_slice(),
            b"GlobalValue = 'File01';\n",
            b"",
            b"\xef\xbb\xbfexport {}\n",
            &[0x47],
        ] {
            assert_eq!(Rule::TransportStream.classify(bytes), FileKind::File);
        }
        assert_eq!(
            Rule::TransportStream.classify(&vec![b'G'; PROBE_BYTES]),
            FileKind::File
        );
        let mut invalid = packets(188, 0);
        for index in 0..6 {
            invalid[index * 188 + 3] = 0;
        }
        assert_eq!(Rule::TransportStream.classify(&invalid), FileKind::File);
        assert_eq!(
            Rule::TransportStream.classify(&packets(188, 0)[..188 * 4]),
            FileKind::File
        );
    }

    #[test]
    fn recognition_never_looks_beyond_the_prefix_budget() {
        let mut bytes = vec![0; PROBE_BYTES];
        bytes.extend(packets(188, 0));
        assert_eq!(Rule::TransportStream.classify(&bytes), FileKind::File);
    }

    #[test]
    fn ambiguous_generic_files_do_not_gain_executable_behavior() {
        assert!(!FileKind::File.can_be_program_with_extension(Some("ts")));
        assert!(!FileKind::File.can_be_program_with_extension(Some("TS")));
        assert!(FileKind::File.can_be_program_with_extension(Some("js")));
        assert!(FileKind::File.can_be_program_with_extension(None));
        assert!(FileKind::Application.can_be_program_with_extension(Some("sh")));
        assert!(!FileKind::Video.can_be_program_with_extension(Some("mp4")));
    }
}
