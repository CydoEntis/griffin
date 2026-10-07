//! Keeps a run's whole process tree together so stopping it, or quitting Glyph,
//! takes down everything the command started, not only the shell (R27).
//!
//! Windows: the child starts suspended, joins a job object that kills every
//! process in it when the job's last handle closes, and only then runs, so nothing
//! it starts can escape the job. Unix: the child leads a process group of its own
//! and stopping signals the whole group.

use std::io;

use tokio::process::{Child, Command};

/// The process tree of one run. Dropping it kills the tree too.
#[derive(Debug)]
pub struct ProcessTree {
    #[cfg(windows)]
    job: std::os::windows::io::OwnedHandle,
    #[cfg(unix)]
    group: i32,
}

/// Sets up `cmd` so the child it starts can be put in a tree; call before spawning.
pub fn prepare(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
        // CREATE_NO_WINDOW: a console of the child's own, hidden, so nothing it
        // runs can write to Glyph's console behind the screen. CREATE_SUSPENDED:
        // the child must not run, or start anything, before it's in the job.
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    }
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
}

impl ProcessTree {
    /// Puts `child`, started from a command passed to `prepare`, in a tree and lets
    /// it run. On failure the child is killed, so it never runs outside a tree.
    pub fn adopt(child: &mut Child) -> io::Result<Self> {
        let tree = Self::adopt_inner(child);
        if tree.is_err() {
            let _ = child.start_kill();
        }
        tree
    }

    #[cfg(windows)]
    fn adopt_inner(child: &mut Child) -> io::Result<Self> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("the child exited before it ran"))?;
        // SAFETY: both pointers may be null: no security attributes, no name.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a job handle just returned to us and owned by nothing
        // else, so `OwnedHandle` may close it.
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };

        // SAFETY: all-zero is a valid "no limits" value for this plain C struct.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of_val(&limits))
            .map_err(|_| io::Error::other("job limits struct too large"))?;
        // SAFETY: `job` is open, and the pointer and size describe `limits`, the
        // struct this information class expects.
        let set = unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `job` and `process` are open handles; tokio keeps the child's
        // handle open while `child` lives.
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `process` is an open handle to the suspended child. Resuming the
        // whole process saves digging out its main thread's handle, which std
        // doesn't keep.
        if unsafe { NtResumeProcess(process) } < 0 {
            return Err(io::Error::other("could not resume the child"));
        }
        Ok(ProcessTree { job })
    }

    #[cfg(unix)]
    fn adopt_inner(child: &mut Child) -> io::Result<Self> {
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("the child exited before it ran"))?;
        // `prepare` made the child lead a group whose id is its pid.
        let group = i32::try_from(pid).map_err(|_| io::Error::other("pid out of range"))?;
        Ok(ProcessTree { group })
    }

    /// Kills every process in the tree. Safe to call more than once.
    pub fn kill(&self) {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;
            // SAFETY: `job` stays open for as long as `self` lives.
            unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) };
        }
        #[cfg(unix)]
        {
            // SAFETY: killpg only sends a signal; a group that's already gone
            // fails with ESRCH, which is fine to ignore.
            unsafe { libc::killpg(self.group, libc::SIGKILL) };
        }
    }
}

impl Drop for ProcessTree {
    // Windows needs nothing here: closing the job's last handle kills the tree.
    // Unix has no such handle, so a dropped tree, or Glyph quitting, kills it.
    fn drop(&mut self) {
        #[cfg(unix)]
        self.kill();
    }
}

#[cfg(windows)]
#[link(name = "ntdll")]
unsafe extern "system" {
    /// Resumes every thread of a process suspended with CREATE_SUSPENDED.
    fn NtResumeProcess(process: windows_sys::Win32::Foundation::HANDLE) -> i32;
}
