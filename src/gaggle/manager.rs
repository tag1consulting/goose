//use crate::{GooseAttack, GooseConfiguration, GooseUserCommand, CANCELED, SHUTDOWN_GAGGLE};
use crate::{GooseAttack, GooseError};

impl GooseAttack {
    /// Main manager loop.
    pub(crate) async fn manager_main(
        mut self,
    ) -> Result<GooseAttack, GooseError> {
        // The GooseAttackRunState is used while spawning and running the
        // GooseUser threads that generate the load test.
        // @TODO: should this be replaced with a GooseAttackManagerState ?
        let mut goose_attack_run_state = self
            .initialize_attack()
            .await
            .expect("failed to initialize GooseAttackRunState");
        
        assert!(goose_attack_run_state.controller_channel_rx.is_some());

        Ok(self)
    }
}