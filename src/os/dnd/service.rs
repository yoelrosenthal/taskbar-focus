//! Restarting the per-user Windows Push Notifications service.
//!
//! Writing the CloudStore blob on its own changes nothing the user can see:
//! `WpnUserService_<luid>` keeps its copy of the quiet-hours state in memory.
//! Bouncing the service makes it re-read the registry and publish the new
//! state, which is the step that actually turns DND on or off.
//!
//! This is the same remedy used by admin scripts and community toggles. It
//! needs no elevation - a per-user service instance is controllable by the user
//! who owns it.
//!
//! The service name has a per-logon suffix (`WpnUserService_1cbf8f`), so we
//! enumerate rather than guess it.

use crate::os::registry::wide;
use windows::core::PCWSTR;
use windows::Win32::System::Services::*;

/// Per-user service instance: `SERVICE_USER_SERVICE | SERVICE_USERSERVICE_INSTANCE`
/// plus the own/shared-process bits.
const USER_SERVICE_TYPES: u32 = 0xF0;
const PREFIX: &str = "WpnUserService_";

/// Stop then start every `WpnUserService_*` instance.
///
/// Returns the number of instances successfully restarted. We deliberately
/// stop-and-start rather than leaving the service down on failure: a stopped
/// notification service is a much worse state to strand the user in than an
/// unapplied DND change.
pub fn restart_notification_service() -> usize {
    let names = enumerate_user_services();
    let mut ok = 0;
    for name in names {
        if restart_one(&name) {
            ok += 1;
        }
    }
    ok
}

fn enumerate_user_services() -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let Ok(scm) = OpenSCManagerW(
            None,
            None,
            SC_MANAGER_ENUMERATE_SERVICE | SC_MANAGER_CONNECT,
        ) else {
            return out;
        };
        let mut needed = 0u32;
        let mut returned = 0u32;
        let mut resume = 0u32;

        let _ = EnumServicesStatusExW(
            scm,
            SC_ENUM_PROCESS_INFO,
            ENUM_SERVICE_TYPE(USER_SERVICE_TYPES),
            SERVICE_STATE_ALL,
            None,
            &mut needed,
            &mut returned,
            Some(&mut resume),
            None,
        );
        if needed > 0 {
            let mut buf = vec![0u8; needed as usize];
            if EnumServicesStatusExW(
                scm,
                SC_ENUM_PROCESS_INFO,
                ENUM_SERVICE_TYPE(USER_SERVICE_TYPES),
                SERVICE_STATE_ALL,
                Some(&mut buf),
                &mut needed,
                &mut returned,
                Some(&mut resume),
                None,
            )
            .is_ok()
            {
                let entries = std::slice::from_raw_parts(
                    buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW,
                    returned as usize,
                );
                for e in entries {
                    if let Ok(name) = e.lpServiceName.to_string() {
                        if name.starts_with(PREFIX) {
                            out.push(name);
                        }
                    }
                }
            }
        }
        let _ = CloseServiceHandle(scm);
    }
    out
}

fn restart_one(name: &str) -> bool {
    unsafe {
        let Ok(scm) = OpenSCManagerW(None, None, SC_MANAGER_CONNECT) else {
            return false;
        };
        let w = wide(name);
        let svc = OpenServiceW(
            scm,
            PCWSTR(w.as_ptr()),
            SERVICE_START | SERVICE_STOP | SERVICE_QUERY_STATUS,
        );
        let result = match svc {
            Ok(svc) => {
                let mut status = SERVICE_STATUS::default();
                let _ = ControlService(svc, SERVICE_CONTROL_STOP, &mut status);
                wait_for(svc, SERVICE_STOPPED);

                for _ in 0..20 {
                    if StartServiceW(svc, None).is_ok() {
                        break;
                    }
                    if current_state(svc) == Some(SERVICE_RUNNING) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                let ok = wait_for(svc, SERVICE_RUNNING);
                let _ = CloseServiceHandle(svc);
                ok
            }
            Err(_) => false,
        };
        let _ = CloseServiceHandle(scm);
        result
    }
}

unsafe fn current_state(svc: SC_HANDLE) -> Option<SERVICE_STATUS_CURRENT_STATE> {
    let mut st = SERVICE_STATUS::default();
    QueryServiceStatus(svc, &mut st)
        .ok()
        .map(|_| st.dwCurrentState)
}

/// Bounded wait so a wedged service cannot hang the UI thread forever.
unsafe fn wait_for(svc: SC_HANDLE, want: SERVICE_STATUS_CURRENT_STATE) -> bool {
    for _ in 0..50 {
        match current_state(svc) {
            Some(s) if s == want => return true,
            None => return false,
            _ => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    false
}
