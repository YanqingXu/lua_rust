//! Injectable lifecycle decisions; these booleans are not real process observations.
// ANCHOR: timeout_lifecycle
pub trait Lifecycle {
    fn kill(&mut self) -> Result<(), String>;
    fn wait(&mut self) -> Result<(), String>;
}

pub type Stop = fn(&mut dyn Lifecycle) -> Result<(), String>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Boundary {
    pub steps: Vec<&'static str>,
    pub owned: bool,
    pub wait_succeeded: bool,
    // Real adapter probes only a no-kill hook; try_wait(Some) would itself reap.
    pub alive_probe: Option<bool>,
}
// ANCHOR_END: timeout_lifecycle

pub struct FakeChild {
    pub alive: bool,
    pub owned: bool,
    pub reaped: bool,
    pub wait_succeeded: bool,
    pub steps: Vec<&'static str>,
    pub kill_race: bool,
    pub fail_wait_once: bool,
}

impl FakeChild {
    pub fn new(kill_race: bool, fail_wait_once: bool) -> Self {
        Self {
            alive: true,
            owned: true,
            reaped: false,
            wait_succeeded: false,
            steps: vec![],
            kill_race,
            fail_wait_once,
        }
    }

    pub fn boundary(&self) -> Boundary {
        Boundary {
            steps: self.steps.clone(),
            owned: self.owned,
            wait_succeeded: self.wait_succeeded,
            alive_probe: Some(self.alive),
        }
    }

    pub fn backstop(&mut self) -> Result<(), String> {
        if self.owned {
            let _ = self.kill();
            self.wait()?;
        }
        Ok(())
    }
}

impl Lifecycle for FakeChild {
    fn kill(&mut self) -> Result<(), String> {
        self.steps.push("kill");
        if self.kill_race || !self.alive {
            self.alive = false;
            return Err("already exited".to_owned());
        }
        self.alive = false;
        Ok(())
    }

    fn wait(&mut self) -> Result<(), String> {
        self.steps.push("wait");
        if self.fail_wait_once {
            self.fail_wait_once = false;
            return Err("injected wait failure".to_owned());
        }
        if self.alive {
            return Err("model wait would block on a running child".to_owned());
        }
        self.reaped = true;
        self.wait_succeeded = true;
        self.owned = false;
        Ok(())
    }
}

#[derive(Debug)]
pub struct Scenario {
    pub hook_result: Result<(), String>,
    pub at_return: Boundary,
    pub backstop_steps: Vec<&'static str>,
    pub final_owned: bool,
    pub final_reaped: bool,
}

pub fn run(stop: Stop, kill_race: bool, fail_wait_once: bool) -> Result<Scenario, String> {
    let mut child = FakeChild::new(kill_race, fail_wait_once);
    let hook_result = stop(&mut child);
    let at_return = child.boundary();
    let before_backstop = child.steps.len();
    child.backstop()?;
    Ok(Scenario {
        hook_result,
        at_return,
        backstop_steps: child.steps[before_backstop..].to_vec(),
        final_owned: child.owned,
        final_reaped: child.reaped,
    })
}

pub fn already_reaped(stop: Stop) -> Result<Boundary, String> {
    let mut child = FakeChild::new(false, false);
    // try_wait(Some) already reaps: release owner, then skip timeout cleanup entirely.
    child.steps.push("try_wait Some");
    child.alive = false;
    child.owned = false;
    child.reaped = true;
    if child.owned {
        stop(&mut child)?;
    }
    Ok(child.boundary())
}
