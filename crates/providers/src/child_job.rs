//! Keep an engine and every command it starts in one Windows process tree.
//!
//! The engines still enforce the filesystem policy themselves (the worktree
//! plus explicitly supplied build-cache roots).  A Job Object supplies the
//! other half of the boundary on Windows: descendants cannot outlive the
//! engine by detaching, and killing/dropping a turn tears down its whole tree.

use std::io;

#[cfg(windows)]
pub(crate) struct ChildJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(not(windows))]
pub(crate) struct ChildJob;

#[cfg(windows)]
impl ChildJob {
    /// Put `pid` in a kill-on-close Job Object. Keep the returned value alive
    /// for exactly as long as the engine child is allowed to live.
    pub(crate) fn assign(pid: u32) -> io::Result<Self> {
        use std::mem::{size_of, zeroed};
        use std::ptr::null;
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        use windows_sys::Win32::System::Threading::{
            CreateJobObjectW, OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };

        // SAFETY: all handles and pointers below are created by Win32, checked
        // before use, and closed on every error path or by `Drop`.
        unsafe {
            let job = CreateJobObjectW(null(), null());
            if job.is_null() {
                return Err(io::Error::from_raw_os_error(GetLastError() as i32));
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                let error = io::Error::from_raw_os_error(GetLastError() as i32);
                CloseHandle(job);
                return Err(error);
            }
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                let error = io::Error::from_raw_os_error(GetLastError() as i32);
                CloseHandle(job);
                return Err(error);
            }
            let assigned = AssignProcessToJobObject(job, process);
            let error =
                (assigned == 0).then(|| io::Error::from_raw_os_error(GetLastError() as i32));
            CloseHandle(process);
            if let Some(error) = error {
                CloseHandle(job);
                return Err(error);
            }
            Ok(Self(job))
        }
    }
}

#[cfg(not(windows))]
impl ChildJob {
    pub(crate) fn assign(_pid: u32) -> io::Result<Self> {
        Ok(Self)
    }
}

#[cfg(windows)]
impl Drop for ChildJob {
    fn drop(&mut self) {
        // SAFETY: the constructor gives this value sole ownership of the
        // checked handle. Closing it also terminates any remaining descendants.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}
