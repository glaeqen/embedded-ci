//! Module providing means of validation for job descriptors.

use std::collections::HashMap;

use crate::{ProbeSerial, RunOn, Target, Targets, UnordEqVec, Uuid};
use core::time::Duration;
use serde::{Deserialize, Serialize};

/// Job
///
/// Root structure representing a concept of a job in embedded CI.
///
/// An abstraction derived from the [`JobDesc`] which contains identifiers and concrete targets to run the tasks on
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// Job identifier
    pub id: Uuid,
    /// Tasks that make up a job
    pub tasks: Vec<Task>,
    /// Global timeout for a job
    ///
    /// Used as an upper limit for how long a job can occupy the server
    pub timeout: Duration,
}

impl Job {
    /// Constructor
    ///
    /// Validates coherency of a descriptor against the available targets
    pub fn from_desc(desc: JobDesc, available_targets: &Targets) -> Result<Self, ValidationErrors> {
        Ok(Self {
            id: Uuid::new_v4(),
            tasks: validate_tasks_coherency(&desc.tasks, available_targets)?,
            timeout: Duration::from_secs(desc.timeout_secs as _),
        })
    }
}

/// Task
///
/// Part of the [`Job`]
///
/// An abstraction derived from the [`TaskDesc`] (as part of the [`Job`]
/// construction) which contains identifiers and concrete targets to run the
/// tasks on
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Task identifier
    pub id: Uuid,
    /// Targets that should be involved as part of this task
    pub targets: Vec<Target>,
    /// Deserialized ELF binary to be run on all the `targets`
    #[serde(skip)]
    pub binary: Vec<u8>,
}

impl Task {
    fn from_desc(targets: Vec<Target>, binary: Vec<u8>) -> Self {
        Self {
            id: Uuid::new_v4(),
            targets,
            binary,
        }
    }
}

/// Result of a job
///
/// Contains details of every single run of every single task which was part of the job of `id`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    /// Identifier matching the corresponding [`Job::id`]
    pub id: Uuid,
    /// Results of individual tasks that make up this job
    pub tasks: Vec<TaskResult>,
}

impl JobResult {
    /// Constructor of an empty job result
    ///
    /// Has to be filled in afterwards with actual results
    pub fn empty_from_job(job: &Job) -> Self {
        let mut job_result = Self {
            id: job.id,
            tasks: Vec::new(),
        };
        for task in job.tasks.iter() {
            let mut task_result = TaskResult {
                id: task.id,
                runs: Vec::new(),
            };
            for target in task.targets.iter() {
                let run_result = RunResult {
                    target: target.clone(),
                    result: Default::default(),
                };
                task_result.runs.push(run_result);
            }
            job_result.tasks.push(task_result);
        }
        job_result
    }

    /// Task result accessor by id
    pub fn task_mut_by_id(&mut self, id: Uuid) -> Option<&mut TaskResult> {
        self.tasks.iter_mut().find(|task| id == task.id)
    }
}

/// Result of a task
///
/// Part of the [`JobResult`]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// Identifier matching the corresponding [`Task::id`]
    pub id: Uuid,
    /// Runs that correspond to all the targets that the ELF was executed on
    pub runs: Vec<RunResult>,
}

impl TaskResult {
    /// Run accessor that filters by the corresponding target's probe serial
    pub fn run_mut_by_probe_serial(
        &mut self,
        probe_serial: &ProbeSerial,
    ) -> Option<&mut RunResult> {
        self.runs
            .iter_mut()
            .find(|run| &run.target.probe_serial == probe_serial)
    }
}

/// Result of a run
///
/// Run corresponds to a single target on which the task was run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    /// Target on which given run was run
    pub target: Target,
    /// Results of a run
    pub result: RunResultDetails,
}

/// Details of a given run
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunResultDetails {
    /// Given run has failed
    ///
    /// That means that the test firmware execution didn't reach the `BKPT` instruction before the [`Job::timeout`].
    Failure {
        /// Stringified error returned by a runner
        error: String,
    },
    /// Given run has succeeded
    ///
    /// That means that the test firmware execution reached the `BKPT` instruction before the [`Job::timeout`].
    Success {
        /// `defmt` logs captured as part of the run
        logs: Vec<String>,
    },
}

impl Default for RunResultDetails {
    fn default() -> Self {
        Self::Failure {
            error: "Never set, possibly timed out".into(),
        }
    }
}

/// A job specification for a run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobDesc {
    /// Collections of tasks to be run simulatenously
    pub tasks: Vec<TaskDesc>,
    /// Timeout of the job in seconds.
    pub timeout_secs: u32,
}

/// A task specification for a run. It is responsible for
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskDesc {
    /// On which embedded targets should this task run on.
    pub run_on: Vec<RunOn>,
    /// The ELF file holding the binary and debug symbols.
    pub binary_b64: String,
}

/// Error aggregating all found validation errors
#[derive(thiserror::Error, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
#[error("Validation failed: {}", errors.iter().fold(String::new(), |a, b| format!("{a}\n\n{b}")))]
pub struct ValidationErrors {
    errors: UnordEqVec<ValidationError>,
}

impl ValidationErrors {
    /// Constructor
    pub fn new(errors: impl Into<UnordEqVec<ValidationError>>) -> Self {
        let errors = errors.into();
        Self { errors }
    }
}

impl core::ops::Deref for ValidationErrors {
    type Target = UnordEqVec<ValidationError>;

    fn deref(&self) -> &Self::Target {
        &self.errors
    }
}

impl core::ops::DerefMut for ValidationErrors {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.errors
    }
}

/// Error reflecting a single validation error found among defined tasks
#[derive(thiserror::Error, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationError {
    #[error("Cannot resolve the target for an entry: {entry}")]
    /// Target requested by the entry does not exist on the server
    TargetNotAvailable {
        /// Offending entry position in the JSON [`JobDesc`]
        entry: String,
    },
    /// Multiple entries resolve to the same target
    #[error("Multiple entries resolve to the same target (probe serial: '{}'): {}. Must be unique", target.probe_serial, entries.join(", "))]
    TargetIsNotUnique {
        /// Resolved target
        target: Target,
        /// Offending entries positions in the JSON [`JobDesc`]
        entries: UnordEqVec<String>,
    },
    /// Decoding of base64 encoded binary failed
    #[error("Decoding of base64 encoded binary failed for an entry: {entry}: {error_details}")]
    Base64DecodingFailed {
        /// Offending entry position in the JSON [`JobDesc`]
        entry: String,
        /// Details of the problem
        error_details: String,
    },
    /// Given task does not define any targets
    #[error("Given task does not define any targets")]
    NoTargetChosen {
        /// Offending entry position in the JSON [`JobDesc`]
        entry: String,
    },
}

/// Validate tasks coherency, that is
///
/// - if all requested targets exist
/// - if all requested targets are unique aka not a single entry resolves to the
///   same target more than once
fn validate_tasks_coherency(
    task_descs: impl AsRef<[TaskDesc]>,
    available_targets: &Targets,
) -> Result<Vec<Task>, ValidationErrors> {
    let mut errors = ValidationErrors::default();
    let mut position_target_map: Vec<(String, Option<Target>)> = Vec::new();
    let mut tasks = Vec::new();
    for (index_t, task_desc) in task_descs.as_ref().iter().enumerate() {
        let mut targets = Vec::new();
        let mut at_least_one_target = false;
        for (index_r, run_on) in task_desc.run_on.iter().enumerate() {
            match run_on {
                RunOn::ProbeSerials(probe_serials) => {
                    for (index_rr, probe_serial) in probe_serials.iter().enumerate() {
                        at_least_one_target |= true;
                        let value = available_targets
                            .find_by_probe_serial(probe_serial)
                            .cloned();
                        let key = format!(
                            "{} @ tasks.{}.run_on.{}.probe_serials.{}",
                            probe_serial, index_t, index_r, index_rr
                        );
                        if let Some(target) = value.clone() {
                            targets.push(target);
                        }
                        position_target_map.push((key, value));
                    }
                }
                RunOn::ProbeAliases(probe_aliases) => {
                    for (index_rr, probe_alias) in probe_aliases.iter().enumerate() {
                        at_least_one_target |= true;
                        let value = available_targets.find_by_probe_alias(probe_alias).cloned();
                        let key = format!(
                            "{} @ tasks.{}.run_on.{}.probe_aliases.{}",
                            probe_alias, index_t, index_r, index_rr
                        );
                        if let Some(target) = value.clone() {
                            targets.push(target);
                        }
                        position_target_map.push((key, value));
                    }
                }
                RunOn::Targets(target_names) => {
                    for (index_rr, target_name) in target_names.iter().enumerate() {
                        at_least_one_target |= true;
                        let value = available_targets.find_by_target_name(target_name).cloned();
                        let key = format!(
                            "{} @ tasks.{}.run_on.{}.targets.{}",
                            target_name, index_t, index_r, index_rr
                        );
                        if let Some(target) = value.clone() {
                            targets.push(target);
                        }
                        position_target_map.push((key, value));
                    }
                }
                RunOn::Groups(groups) => {
                    for (index_rr, group) in groups.iter().enumerate() {
                        at_least_one_target |= true;
                        let key = format!(
                            "{} @ tasks.{}.run_on.{}.groups.{}",
                            group, index_t, index_r, index_rr
                        );
                        if available_targets
                            .find_by_group(group)
                            .map(|value| {
                                targets.push(value.clone());
                                position_target_map.push((key.clone(), Some(value.clone())));
                            })
                            .count()
                            == 0
                        {
                            position_target_map.push((key, None));
                        }
                    }
                }
            }
        }
        if !at_least_one_target {
            errors.push(ValidationError::NoTargetChosen {
                entry: format!("tasks.{}.run_on", index_t),
            });
        }
        match base64::decode(&task_desc.binary_b64) {
            Ok(binary) => tasks.push(Task::from_desc(targets, binary)),
            Err(e) => errors.push(ValidationError::Base64DecodingFailed {
                entry: format!("tasks.{}.binary_b64", index_t),
                error_details: e.to_string(),
            }),
        };
    }

    position_target_map
        .iter()
        .filter(|&(_, v)| v.is_none())
        .map(|(k, _)| ValidationError::TargetNotAvailable { entry: k.clone() })
        .for_each(|v| errors.push(v));

    let mut target_position_map = HashMap::<ProbeSerial, (Target, Vec<String>)>::new();
    for (position, target) in position_target_map.iter() {
        let target = match target {
            Some(target) => target.clone(),
            None => continue,
        };
        target_position_map
            .entry(target.probe_serial.clone())
            .or_insert_with(|| (target, Vec::new().into()))
            .1
            .push(position.clone());
    }

    for (target, positions) in target_position_map.into_values() {
        if positions.len() > 1 {
            errors.push(ValidationError::TargetIsNotUnique {
                target,
                entries: positions.into(),
            });
        }
    }

    if errors.is_empty() {
        Ok(tasks)
    } else {
        Err(errors.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProbeAlias, ProbeSerial, TargetGroup, TargetName};

    fn get_available_targets() -> Vec<Target> {
        vec![
            Target {
                probe_serial: ProbeSerial("PROBE_SERIAL_1".into()),
                probe_alias: ProbeAlias("PROBE_ALIAS_1".into()),
                target_name: TargetName("TARGET_1".into()),
                groups: vec![TargetGroup("GROUP_A".into())].into(),
            },
            Target {
                probe_serial: ProbeSerial("PROBE_SERIAL_2".into()),
                probe_alias: ProbeAlias("PROBE_ALIAS_2".into()),
                target_name: TargetName("TARGET_2".into()),
                groups: vec![TargetGroup("GROUP_A".into())].into(),
            },
            Target {
                probe_serial: ProbeSerial("PROBE_SERIAL_3".into()),
                probe_alias: ProbeAlias("PROBE_ALIAS_3".into()),
                target_name: TargetName("TARGET_3".into()),
                groups: vec![TargetGroup("GROUP_A".into())].into(),
            },
            Target {
                probe_serial: ProbeSerial("PROBE_SERIAL_4".into()),
                probe_alias: ProbeAlias("PROBE_ALIAS_4".into()),
                target_name: TargetName("TARGET_4".into()),
                groups: vec![TargetGroup("GROUP_B".into())].into(),
            },
            Target {
                probe_serial: ProbeSerial("PROBE_SERIAL_5".into()),
                probe_alias: ProbeAlias("PROBE_ALIAS_5".into()),
                target_name: TargetName("TARGET_5".into()),
                groups: vec![TargetGroup("GROUP_B".into())].into(),
            },
        ]
    }

    #[test]
    fn valid_set_of_tasks() {
        let tasks = vec![TaskDesc {
            binary_b64: "bm90X2NoZWNrZWQ=".into(),
            run_on: vec![
                RunOn::ProbeSerials(vec![ProbeSerial("PROBE_SERIAL_1".into())]),
                RunOn::ProbeAliases(vec![ProbeAlias("PROBE_ALIAS_2".into())]),
                RunOn::Targets(vec![TargetName("TARGET_3".into())]),
                RunOn::Groups(vec![TargetGroup("GROUP_B".into())]),
            ],
        }];

        let all_targets = get_available_targets();
        let tasks = validate_tasks_coherency(&tasks, &all_targets.clone().into()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            UnordEqVec::from(tasks[0].targets.clone()),
            UnordEqVec::from(all_targets)
        );
    }

    #[test]
    fn invalid_b64_encoded_binary() {
        let tasks = vec![
            TaskDesc {
                binary_b64: "c2hvdWxkX3dvcms=".into(),
                run_on: vec![RunOn::ProbeSerials(vec![ProbeSerial(
                    "PROBE_SERIAL_1".into(),
                )])],
            },
            TaskDesc {
                binary_b64: "ooops".into(),
                run_on: vec![RunOn::ProbeAliases(vec![ProbeAlias(
                    "PROBE_ALIAS_2".into(),
                )])],
            },
        ];
        let result = validate_tasks_coherency(&tasks, &get_available_targets().into());
        let expected = ValidationErrors::new(vec![ValidationError::Base64DecodingFailed {
            entry: "tasks.1.binary_b64".into(),
            error_details: base64::DecodeError::InvalidLength.to_string(),
        }]);
        match result {
            Ok(_) => panic!("expected: {:?}, found Ok", expected),
            Err(result) => assert_eq!(result, expected),
        }
    }

    #[test]
    fn target_duplicated_within_task() {
        let tasks = vec![TaskDesc {
            binary_b64: "bm90X2NoZWNrZWQ=".into(),
            run_on: vec![
                RunOn::ProbeSerials(vec![ProbeSerial("PROBE_SERIAL_2".into())]),
                RunOn::ProbeAliases(vec![ProbeAlias("PROBE_ALIAS_2".into())]),
                RunOn::Targets(vec![TargetName("TARGET_2".into())]),
                RunOn::Groups(vec![TargetGroup("GROUP_B".into())]),
            ],
        }];
        let result = validate_tasks_coherency(&tasks, &get_available_targets().into());
        let expected = ValidationErrors::new(vec![ValidationError::TargetIsNotUnique {
            target: Target {
                probe_serial: ProbeSerial("PROBE_SERIAL_2".into()),
                probe_alias: ProbeAlias("PROBE_ALIAS_2".into()),
                target_name: TargetName("TARGET_2".into()),
                groups: vec![TargetGroup("GROUP_A".into())].into(),
            },
            entries: vec![
                "TARGET_2 @ tasks.0.run_on.2.targets.0".into(),
                "PROBE_SERIAL_2 @ tasks.0.run_on.0.probe_serials.0".into(),
                "PROBE_ALIAS_2 @ tasks.0.run_on.1.probe_aliases.0".into(),
            ]
            .into(),
        }]);
        // println!("{}", serde_json::to_string_pretty(&expected).unwrap());
        // panic!();
        match result {
            Ok(_) => panic!("expected: {:?}, found Ok", expected),
            Err(result) => assert_eq!(result, expected),
        }
    }

    #[test]
    fn target_duplicated_between_tasks() {
        let tasks = vec![
            TaskDesc {
                binary_b64: "bm90X2NoZWNrZWQ=".into(),
                run_on: vec![RunOn::ProbeSerials(vec![ProbeSerial(
                    "PROBE_SERIAL_2".into(),
                )])],
            },
            TaskDesc {
                binary_b64: "bm90X2NoZWNrZWQ=".into(),
                run_on: vec![RunOn::ProbeAliases(vec![ProbeAlias(
                    "PROBE_ALIAS_2".into(),
                )])],
            },
            TaskDesc {
                binary_b64: "bm90X2NoZWNrZWQ=".into(),
                run_on: vec![RunOn::Targets(vec![TargetName("TARGET_2".into())])],
            },
        ];
        let result = validate_tasks_coherency(&tasks, &get_available_targets().into());
        let expected = ValidationErrors::new(vec![ValidationError::TargetIsNotUnique {
            target: Target {
                probe_serial: ProbeSerial("PROBE_SERIAL_2".into()),
                probe_alias: ProbeAlias("PROBE_ALIAS_2".into()),
                target_name: TargetName("TARGET_2".into()),
                groups: vec![TargetGroup("GROUP_A".into())].into(),
            },
            entries: vec![
                "TARGET_2 @ tasks.2.run_on.0.targets.0".into(),
                "PROBE_SERIAL_2 @ tasks.0.run_on.0.probe_serials.0".into(),
                "PROBE_ALIAS_2 @ tasks.1.run_on.0.probe_aliases.0".into(),
            ]
            .into(),
        }]);
        match result {
            Ok(_) => panic!("expected: {:?}, found Ok", expected),
            Err(result) => assert_eq!(result, expected),
        }
    }

    #[test]
    fn target_duplicated_via_group() {
        let tasks = vec![TaskDesc {
            binary_b64: "bm90X2NoZWNrZWQ=".into(),
            run_on: vec![
                RunOn::ProbeSerials(vec![ProbeSerial("PROBE_SERIAL_1".into())]),
                RunOn::ProbeAliases(vec![ProbeAlias("PROBE_ALIAS_2".into())]),
                RunOn::Targets(vec![TargetName("TARGET_3".into())]),
                RunOn::Groups(vec![TargetGroup("GROUP_A".into())]),
            ],
        }];

        let result = validate_tasks_coherency(&tasks, &get_available_targets().into());
        let expected = ValidationErrors::new(vec![
            ValidationError::TargetIsNotUnique {
                target: Target {
                    probe_serial: ProbeSerial("PROBE_SERIAL_1".into()),
                    probe_alias: ProbeAlias("PROBE_ALIAS_1".into()),
                    target_name: TargetName("TARGET_1".into()),
                    groups: vec![TargetGroup("GROUP_A".into())].into(),
                },
                entries: vec![
                    "PROBE_SERIAL_1 @ tasks.0.run_on.0.probe_serials.0".into(),
                    "GROUP_A @ tasks.0.run_on.3.groups.0".into(),
                ]
                .into(),
            },
            ValidationError::TargetIsNotUnique {
                target: Target {
                    probe_serial: ProbeSerial("PROBE_SERIAL_2".into()),
                    probe_alias: ProbeAlias("PROBE_ALIAS_2".into()),
                    target_name: TargetName("TARGET_2".into()),
                    groups: vec![TargetGroup("GROUP_A".into())].into(),
                },
                entries: vec![
                    "PROBE_ALIAS_2 @ tasks.0.run_on.1.probe_aliases.0".into(),
                    "GROUP_A @ tasks.0.run_on.3.groups.0".into(),
                ]
                .into(),
            },
            ValidationError::TargetIsNotUnique {
                target: Target {
                    probe_serial: ProbeSerial("PROBE_SERIAL_3".into()),
                    probe_alias: ProbeAlias("PROBE_ALIAS_3".into()),
                    target_name: TargetName("TARGET_3".into()),
                    groups: vec![TargetGroup("GROUP_A".into())].into(),
                },
                entries: vec![
                    "TARGET_3 @ tasks.0.run_on.2.targets.0".into(),
                    "GROUP_A @ tasks.0.run_on.3.groups.0".into(),
                ]
                .into(),
            },
        ]);
        match result {
            Ok(_) => panic!("expected: {:?}, found Ok", expected),
            Err(result) => assert_eq!(result, expected),
        }
    }

    #[test]
    fn target_does_not_exist() {
        let tasks = vec![TaskDesc {
            binary_b64: "bm90X2NoZWNrZWQ=".into(),
            run_on: vec![
                RunOn::ProbeSerials(vec![ProbeSerial("PROBE_SERIAL_1".into())]),
                RunOn::ProbeAliases(vec![ProbeAlias("PROBE_ALIAS_2".into())]),
                RunOn::Targets(vec![TargetName("TARGET_3".into())]),
                RunOn::Groups(vec![TargetGroup("GROUP_C".into())]),
            ],
        }];

        let result = validate_tasks_coherency(&tasks, &get_available_targets().into());
        let expected = ValidationErrors::new(vec![ValidationError::TargetNotAvailable {
            entry: "GROUP_C @ tasks.0.run_on.3.groups.0".into(),
        }]);
        match result {
            Ok(_) => panic!("expected: {:?}, found Ok", expected),
            Err(result) => assert_eq!(result, expected),
        }
    }

    #[test]
    fn target_not_specified() {
        let tasks = vec![
            TaskDesc {
                binary_b64: "bm90X2NoZWNrZWQ=".into(),
                run_on: vec![RunOn::ProbeSerials(vec![ProbeSerial(
                    "PROBE_SERIAL_1".into(),
                )])],
            },
            TaskDesc {
                binary_b64: "bm90X2NoZWNrZWQ=".into(),
                run_on: vec![],
            },
        ];

        let result = validate_tasks_coherency(&tasks, &get_available_targets().into());
        let expected = ValidationErrors::new(vec![ValidationError::NoTargetChosen {
            entry: "tasks.1.run_on".into(),
        }]);
        match result {
            Ok(_) => panic!("expected: {:?}, found Ok", expected),
            Err(result) => assert_eq!(result, expected),
        }
    }
}
