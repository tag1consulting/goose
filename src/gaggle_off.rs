//! The Gaggle hooks a run calls, in a build without the `gaggle` feature:
//! no process is a manager or a worker there, so each does nothing, and the
//! run's code calls them as it does in a build with it.

use crate::config::GooseConfiguration;
use crate::{GooseAttack, GooseAttackRunState, GooseError};

impl GooseConfiguration {
    pub(crate) fn worker_refusals(&self) -> Result<(), GooseError> {
        Ok(())
    }

    pub(crate) fn is_manager(&self) -> bool {
        false
    }

    pub(crate) fn is_worker(&self) -> bool {
        false
    }
}

impl GooseAttack {
    pub(crate) async fn gaggle_worker_join(&mut self) -> Result<bool, GooseError> {
        Ok(true)
    }

    pub(crate) fn gaggle_worker_prepared(&self, _error: Option<&GooseError>) {}

    pub(crate) async fn gaggle_worker_exit(&self) {}

    pub(crate) fn gaggle_worker_holds(&self) -> bool {
        false
    }

    pub(crate) async fn gaggle_worker_idle(
        &mut self,
        _goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        Ok(())
    }

    pub(crate) fn gaggle_watch_user<T>(&self, _user: &tokio::task::JoinHandle<T>) {}

    pub(crate) async fn gaggle_manager_start(&mut self) -> Result<(), GooseError> {
        Ok(())
    }

    pub(crate) async fn gaggle_manager_failed_to_start(&mut self) {}

    pub(crate) async fn gaggle_manager_increase(
        &mut self,
        _goose_attack_run_state: &mut GooseAttackRunState,
        _increase_rate: f32,
    ) -> bool {
        false
    }

    pub(crate) fn gaggle_manager_decrease(
        &self,
        _goose_attack_run_state: &mut GooseAttackRunState,
    ) -> bool {
        false
    }

    pub(crate) async fn gaggle_tick(
        &mut self,
        _goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        Ok(())
    }

    pub(crate) async fn gaggle_end_of_run(
        &mut self,
        _goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        Ok(())
    }

    pub(crate) async fn gaggle_collect_logs(&mut self) -> bool {
        false
    }

    pub(crate) async fn gaggle_shut_down(&mut self) {}

    pub(crate) fn gaggle_shadow_report(&mut self) -> Result<(), GooseError> {
        Ok(())
    }
}
