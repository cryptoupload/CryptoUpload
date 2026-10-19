// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

pub fn harden_process() {
    #[cfg(unix)]
    {
        use rustix::process::{Resource, Rlimit, setrlimit};
        let _ = setrlimit(
            Resource::Core,
            Rlimit {
                current: Some(0),
                maximum: Some(0),
            },
        );
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use rustix::process::{DumpableBehavior, set_dumpable_behavior};
        let _ = set_dumpable_behavior(DumpableBehavior::NotDumpable);
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        let mut config = secmem_proc::Config::default();
        config.set_anti_tracing(false);
        let _ = config.harden_process();
    }
}

pub fn secure_app_data_dir(data_dir: &Path) {
    let dir = data_dir.join("cryptoupload");
    let _ = std::fs::create_dir_all(&dir);
    restrict_to_owner(&dir, 0o700);
}

pub(crate) fn restrict_to_owner(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}
