//! Transaction support for grouping multiple commands.

use std::any::Any;

use super::command::{Command, CommandResult};

/// A transaction groups multiple commands into a single undoable unit.
///
/// When a transaction is undone, all commands within it are undone
/// in reverse order. When redone, they are executed in original order.
pub struct Transaction {
    commands: Vec<Box<dyn Command>>,
    description: String,
}

impl Transaction {
    /// Creates a new empty transaction.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            commands: Vec::new(),
            description: description.into(),
        }
    }

    /// Adds a command to the transaction.
    pub fn add(&mut self, command: Box<dyn Command>) {
        self.commands.push(command);
    }

    /// Returns true if the transaction is empty.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Returns the number of commands in the transaction.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Creates a builder for constructing transactions.
    pub fn builder(description: impl Into<String>) -> TransactionBuilder {
        TransactionBuilder::new(description)
    }
}

impl Command for Transaction {
    fn execute(&mut self, state: &mut dyn Any) -> CommandResult {
        let mut executed = 0;

        for command in &mut self.commands {
            if let Err(e) = command.execute(state) {
                // Rollback executed commands on failure
                for cmd in self.commands[..executed].iter_mut().rev() {
                    let _ = cmd.undo(state);
                }
                return Err(e);
            }
            executed += 1;
        }

        Ok(())
    }

    fn undo(&mut self, state: &mut dyn Any) -> CommandResult {
        // Undo in reverse order
        for command in self.commands.iter_mut().rev() {
            command.undo(state)?;
        }
        Ok(())
    }

    fn redo(&mut self, state: &mut dyn Any) -> CommandResult {
        // Redo in original order
        for command in &mut self.commands {
            command.redo(state)?;
        }
        Ok(())
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn memory_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.commands.iter().map(|c| c.memory_size()).sum::<usize>()
    }

    fn is_significant(&self) -> bool {
        self.commands.iter().any(|c| c.is_significant())
    }
}

/// Builder for constructing transactions fluently.
pub struct TransactionBuilder {
    transaction: Transaction,
}

impl TransactionBuilder {
    /// Creates a new transaction builder.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            transaction: Transaction::new(description),
        }
    }

    /// Adds a command to the transaction.
    pub fn command(mut self, command: Box<dyn Command>) -> Self {
        self.transaction.add(command);
        self
    }

    /// Adds a command if the condition is true.
    pub fn command_if(self, condition: bool, command: Box<dyn Command>) -> Self {
        if condition {
            self.command(command)
        } else {
            self
        }
    }

    /// Builds the transaction.
    pub fn build(self) -> Transaction {
        self.transaction
    }

    /// Builds and executes the transaction immediately.
    pub fn execute(
        self,
        history: &mut super::CommandHistory,
        state: &mut dyn Any,
    ) -> CommandResult {
        if self.transaction.is_empty() {
            return Ok(());
        }
        history.execute(Box::new(self.transaction), state)
    }
}

/// Macro for creating transactions.
#[macro_export]
macro_rules! transaction {
    ($desc:expr => [ $($cmd:expr),* $(,)? ]) => {{
        let mut tx = $crate::commands::Transaction::new($desc);
        $(
            tx.add(Box::new($cmd));
        )*
        tx
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandError;

    struct TestState {
        values: Vec<i32>,
    }

    struct PushCommand {
        value: i32,
    }

    impl Command for PushCommand {
        fn execute(&mut self, state: &mut dyn Any) -> CommandResult {
            let state = state
                .downcast_mut::<TestState>()
                .ok_or_else(|| CommandError::InvalidState("Expected TestState".to_string()))?;
            state.values.push(self.value);
            Ok(())
        }

        fn undo(&mut self, state: &mut dyn Any) -> CommandResult {
            let state = state
                .downcast_mut::<TestState>()
                .ok_or_else(|| CommandError::InvalidState("Expected TestState".to_string()))?;
            state.values.pop();
            Ok(())
        }

        fn description(&self) -> &str {
            "Push value"
        }
    }

    #[test]
    fn test_transaction_execute() {
        let mut state = TestState { values: Vec::new() };

        let mut tx = Transaction::new("Push multiple");
        tx.add(Box::new(PushCommand { value: 1 }));
        tx.add(Box::new(PushCommand { value: 2 }));
        tx.add(Box::new(PushCommand { value: 3 }));

        tx.execute(&mut state).unwrap();

        assert_eq!(state.values, vec![1, 2, 3]);
    }

    #[test]
    fn test_transaction_undo() {
        let mut state = TestState { values: Vec::new() };

        let mut tx = Transaction::new("Push multiple");
        tx.add(Box::new(PushCommand { value: 1 }));
        tx.add(Box::new(PushCommand { value: 2 }));
        tx.add(Box::new(PushCommand { value: 3 }));

        tx.execute(&mut state).unwrap();
        tx.undo(&mut state).unwrap();

        assert!(state.values.is_empty());
    }

    #[test]
    fn test_transaction_builder() {
        let mut state = TestState { values: Vec::new() };

        let mut tx = Transaction::builder("Build and push")
            .command(Box::new(PushCommand { value: 10 }))
            .command(Box::new(PushCommand { value: 20 }))
            .build();

        tx.execute(&mut state).unwrap();

        assert_eq!(state.values, vec![10, 20]);
    }
}
