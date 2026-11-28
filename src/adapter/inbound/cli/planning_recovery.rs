use std::ffi::{OsStr, OsString};
use std::io::Write;

use anyhow::{Result, bail};

use crate::composition::production::legacy_diagnosis::{self, LegacyNamespaceDiagnosis};

pub(super) const USAGE: &str = "Usage: akra planning-recovery diagnose [workspace_dir]";

pub(crate) fn is_command(command: &OsStr) -> bool {
    command == OsStr::new("planning-recovery")
}

pub(super) fn run(args: &[OsString], stdout: &mut impl Write) -> Result<i32> {
    let (action, workspace) = match args {
        [action] => (action, None),
        [action, workspace] => (action, Some(workspace.as_os_str())),
        _ => bail!("{USAGE}"),
    };
    if action != OsStr::new("diagnose") {
        bail!("{USAGE}");
    }
    let workspace = super::resolve_workspace_path(workspace)?;
    super::validate_workspace_path(&workspace).map_err(anyhow::Error::msg)?;
    match legacy_diagnosis::inspect(&workspace)? {
        LegacyNamespaceDiagnosis::Absent => {
            writeln!(stdout, "legacy_namespace: absent")?;
            writeln!(
                stdout,
                "현재 프로필에서 구형 저장 경로를 찾지 못했습니다. 이전에 사용한 AKRA_HOME을 확인하세요."
            )?;
        }
        LegacyNamespaceDiagnosis::Occupied(path) => {
            writeln!(stdout, "legacy_namespace: occupied")?;
            writeln!(stdout, "path: {}", path.display())?;
            writeln!(
                stdout,
                "구형 저장 경로가 이미 사용 중입니다. 같은 경로에 새 Git 저장소를 만들 수 있으므로 경로만으로는 현재 저장소 소유임을 증명할 수 없습니다."
            )?;
            writeln!(
                stdout,
                "기존 데이터를 그대로 보관하세요. DB를 열거나 선택하지 않았고, 복사·가져오기·변경도 하지 않았습니다."
            )?;
        }
        LegacyNamespaceDiagnosis::SameAsCurrentNamespace(path) => {
            writeln!(stdout, "legacy_namespace: overlaps_current_namespace")?;
            writeln!(stdout, "path: {}", path.display())?;
            writeln!(
                stdout,
                "구형 저장 경로가 현재 저장소의 경로 표식과 겹칩니다. 경로만으로는 DB의 소유를 증명할 수 없습니다."
            )?;
            writeln!(
                stdout,
                "기존 데이터를 그대로 보관하세요. DB를 열거나 변경하지 않았습니다."
            )?;
        }
    }
    Ok(0)
}
