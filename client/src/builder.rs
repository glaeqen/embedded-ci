//! Module containing the low level [`JobDesc `] builder

use embedded_ci_common::job::{JobDesc, TaskDesc};
pub use embedded_ci_common::*;

/// Possible errors produced by the [`JobDescBuilder`]
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// No timeout was specified for a job
    #[error("No timeout was specified for a job")]
    NoTimeout,
    /// Generic error.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    /// No run_ons have been specified for a task
    #[error("No run_ons have been specified for a task")]
    NoRunOns,
    /// No ELF has been specified for a task
    #[error("No ELF has been specified for a task")]
    NoElf,
    /// No tasks has been specified for a job
    #[error("No tasks has been specified for a job")]
    NoTasks,
}

type Result<T> = core::result::Result<T, Error>;

/// Low level [`JobDesc`] builder without any coherency checking capabilities
pub struct JobDescBuilder {
    tasks: Vec<TaskDesc>,
    timeout_secs: Option<u32>,
}

impl JobDescBuilder {
    /// Constructor
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            timeout_secs: None,
        }
    }

    /// Add a new task
    ///
    /// Returns a task builder. In order to finish the task building,
    /// call [`TaskDescBuilder::done`] or [`TaskDescBuilder::build`]
    /// to conclude the parent, job builder.
    pub fn add_task(self) -> TaskDescBuilder {
        TaskDescBuilder::new(self)
    }

    /// Set the timeout of a job
    pub fn set_timeout(mut self, timeout_secs: u32) -> JobDescBuilder {
        self.timeout_secs = Some(timeout_secs);
        self
    }

    /// Finish the job
    pub fn build(self) -> Result<JobDesc> {
        if self.tasks.len() == 0 {
            return Err(Error::NoTasks);
        }
        Ok(JobDesc {
            tasks: self.tasks,
            timeout_secs: self.timeout_secs.ok_or_else(|| Error::NoTimeout)?,
        })
    }
}

/// Low level [`TaskDesc`]s builder without any coherency checking capabilities
pub struct TaskDescBuilder {
    parent_builder: JobDescBuilder,
    elf: Option<Vec<u8>>,
    run_ons: Vec<RunOn>,
}

impl TaskDescBuilder {
    fn new(parent_builder: JobDescBuilder) -> Self {
        Self {
            parent_builder,
            elf: None,
            run_ons: Vec::new(),
        }
    }

    /// Set the ELF executable that is supposed to be run on targets matching specified [`RunOn`]s
    pub fn elf_executable(mut self, elf: Vec<u8>) -> Self {
        self.elf = Some(elf);
        self
    }

    /// Specify the targets which the chosen ELF is supposed to be run on.
    pub fn run_on(mut self, run_on: RunOn) -> Self {
        self.run_ons.push(run_on);
        self
    }

    /// Finish the task
    pub fn done(mut self) -> Result<JobDescBuilder> {
        if self.run_ons.len() == 0 {
            return Err(Error::NoRunOns);
        }
        self.parent_builder.tasks.push(TaskDesc {
            run_on: self.run_ons,
            binary_b64: base64::encode(self.elf.ok_or_else(|| Error::NoElf)?),
        });
        Ok(self.parent_builder)
    }

    /// Finish the job
    pub fn build(self) -> Result<JobDesc> {
        self.done()?.build()
    }
}
