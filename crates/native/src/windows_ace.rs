#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowsAceMutationAction {
    Ignore,
    InspectStandardAllowSid,
    RejectUnrecognizedAllow,
}

#[cfg(any(windows, test))]
pub(crate) fn classify_windows_ace_mutation(
    ace_type: u8,
    mask: u32,
    directory: bool,
) -> WindowsAceMutationAction {
    const DELETE: u32 = 0x0001_0000;
    const FILE_DELETE_CHILD: u32 = 0x0000_0040;
    const FILE_WRITE_DATA: u32 = 0x0000_0002;
    const FILE_APPEND_DATA: u32 = 0x0000_0004;
    const FILE_WRITE_EA: u32 = 0x0000_0010;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
    const WRITE_DAC: u32 = 0x0004_0000;
    const WRITE_OWNER: u32 = 0x0008_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const GENERIC_ALL: u32 = 0x1000_0000;
    let dangerous = if directory {
        DELETE | FILE_DELETE_CHILD | WRITE_DAC | WRITE_OWNER | GENERIC_ALL
    } else {
        DELETE
            | FILE_WRITE_DATA
            | FILE_APPEND_DATA
            | FILE_WRITE_EA
            | FILE_WRITE_ATTRIBUTES
            | WRITE_DAC
            | WRITE_OWNER
            | GENERIC_WRITE
            | GENERIC_ALL
    };
    if mask & dangerous == 0 {
        return WindowsAceMutationAction::Ignore;
    }
    match ace_type {
        0 => WindowsAceMutationAction::InspectStandardAllowSid,
        // Compound, object, and callback allow ACEs need type-specific SID/condition parsing.
        4 | 5 | 9 | 11 => WindowsAceMutationAction::RejectUnrecognizedAllow,
        // Known deny/audit/alarm/mandatory/resource/trust/filter ACEs do not grant access.
        1..=3 | 6..=8 | 10 | 12..=15 | 17..=21 => WindowsAceMutationAction::Ignore,
        _ => WindowsAceMutationAction::RejectUnrecognizedAllow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_ace_classification_distinguishes_file_and_directory_mutation_rights() {
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0000_0002, false),
            WindowsAceMutationAction::InspectStandardAllowSid
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0000_0002, true),
            WindowsAceMutationAction::Ignore,
            "creating a sibling cannot replace an existing executable path"
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0000_0004, true),
            WindowsAceMutationAction::Ignore,
            "creating a sibling directory cannot replace an existing path component"
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x4000_0000, true),
            WindowsAceMutationAction::Ignore,
            "generic directory write does not grant delete-child or ACL replacement"
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0000_0040, true),
            WindowsAceMutationAction::InspectStandardAllowSid
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0004_0000, true),
            WindowsAceMutationAction::InspectStandardAllowSid
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x1000_0000, true),
            WindowsAceMutationAction::InspectStandardAllowSid
        );
        assert_eq!(
            classify_windows_ace_mutation(9, 0x4000_0000, false),
            WindowsAceMutationAction::RejectUnrecognizedAllow
        );
        assert_eq!(
            classify_windows_ace_mutation(1, 0x1000_0000, false),
            WindowsAceMutationAction::Ignore,
            "deny ACEs never grant mutation rights"
        );
        assert_eq!(
            classify_windows_ace_mutation(0xff, 0x1000_0000, false),
            WindowsAceMutationAction::RejectUnrecognizedAllow
        );
    }
}
