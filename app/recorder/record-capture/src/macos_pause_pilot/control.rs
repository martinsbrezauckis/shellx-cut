//! Monotonic command correlation for one private macOS pause owner.

use super::{MacosPauseCommand, MacosPauseCommandRejection};

#[derive(Debug, Default)]
pub(super) struct CommandState {
    last_epoch: u64,
    last_generation: u64,
    last_sealed_physical_generation: Option<u64>,
}

impl CommandState {
    pub(super) fn validate(
        &self,
        command: MacosPauseCommand,
        active_physical_generation: Option<u64>,
        stopped: bool,
        discarded_epochs: &[u64],
    ) -> Result<(), MacosPauseCommandRejection> {
        if stopped {
            return Err(MacosPauseCommandRejection::Stopped);
        }
        let expected_epoch =
            self.last_epoch
                .checked_add(1)
                .ok_or(MacosPauseCommandRejection::EpochOutOfOrder {
                    expected: u64::MAX,
                    received: command.epoch(),
                })?;
        let terminal_epoch_skip =
            accounts_for_discarded_epochs(command, expected_epoch, discarded_epochs);
        if command.epoch() != expected_epoch && !terminal_epoch_skip {
            return Err(MacosPauseCommandRejection::EpochOutOfOrder {
                expected: expected_epoch,
                received: command.epoch(),
            });
        }
        if let Some(generation) = command.generation() {
            let expected_generation = self.last_generation.checked_add(1).ok_or(
                MacosPauseCommandRejection::GenerationOutOfOrder {
                    expected: u64::MAX,
                    received: generation,
                },
            )?;
            if generation != expected_generation {
                return Err(MacosPauseCommandRejection::GenerationOutOfOrder {
                    expected: expected_generation,
                    received: generation,
                });
            }
        }
        match command {
            MacosPauseCommand::Pause {
                physical_generation,
                ..
            } => match active_physical_generation {
                Some(expected) if expected == physical_generation => Ok(()),
                Some(expected) => Err(MacosPauseCommandRejection::PhysicalGenerationMismatch {
                    expected,
                    received: physical_generation,
                }),
                None => Err(MacosPauseCommandRejection::WrongPhase),
            },
            MacosPauseCommand::Resume {
                resume_from_physical_generation,
                ..
            } => match (
                active_physical_generation,
                self.last_sealed_physical_generation,
            ) {
                (None, Some(expected)) if expected == resume_from_physical_generation => Ok(()),
                (None, Some(expected)) => {
                    Err(MacosPauseCommandRejection::PhysicalGenerationMismatch {
                        expected,
                        received: resume_from_physical_generation,
                    })
                }
                _ => Err(MacosPauseCommandRejection::WrongPhase),
            },
            MacosPauseCommand::Stop {
                physical_generation,
                ..
            } => match active_physical_generation.or(self.last_sealed_physical_generation) {
                Some(expected) if expected == physical_generation => Ok(()),
                Some(expected) => Err(MacosPauseCommandRejection::PhysicalGenerationMismatch {
                    expected,
                    received: physical_generation,
                }),
                None => Err(MacosPauseCommandRejection::WrongPhase),
            },
        }
    }

    pub(super) fn accept(&mut self, command: MacosPauseCommand) {
        self.last_epoch = command.epoch();
        if let Some(generation) = command.generation() {
            self.last_generation = generation;
        }
    }

    pub(super) fn note_sealed_physical_generation(&mut self, physical_generation: u64) {
        self.last_sealed_physical_generation = Some(physical_generation);
    }

    pub(super) fn last_sealed_physical_generation(&self) -> Option<u64> {
        self.last_sealed_physical_generation
    }
}

/// Stop dominance may discard a contiguous already-correlated prefix without
/// touching native capture. A terminal epoch may skip only that exact prefix;
/// a Stop at index zero or any gap remains a rejected delayed command.
fn accounts_for_discarded_epochs(
    command: MacosPauseCommand,
    expected_epoch: u64,
    discarded_epochs: &[u64],
) -> bool {
    if !matches!(command, MacosPauseCommand::Stop { .. }) || discarded_epochs.is_empty() {
        return false;
    }
    let mut expected = expected_epoch;
    for discarded in discarded_epochs {
        if *discarded != expected {
            return false;
        }
        let Some(next) = expected.checked_add(1) else {
            return false;
        };
        expected = next;
    }
    command.epoch() == expected
}

#[cfg(test)]
mod tests {
    use super::{CommandState, MacosPauseCommand, MacosPauseCommandRejection};

    fn stop(epoch: u64) -> MacosPauseCommand {
        MacosPauseCommand::Stop {
            epoch,
            physical_generation: 1,
        }
    }

    #[test]
    fn terminal_epoch_skip_must_exactly_match_the_discarded_prefix() {
        let cases: [(&[u64], u64); 3] = [(&[], 2), (&[1, 3], 4), (&[1], 3)];
        for (discarded, received) in cases {
            assert_eq!(
                CommandState::default().validate(stop(received), Some(1), false, discarded),
                Err(MacosPauseCommandRejection::EpochOutOfOrder {
                    expected: 1,
                    received,
                })
            );
        }
        assert_eq!(
            CommandState::default().validate(stop(2), Some(1), false, &[1]),
            Ok(())
        );
    }
}
