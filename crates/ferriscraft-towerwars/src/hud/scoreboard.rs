use rspigot_plugin::{Criteria, DisplaySlot, Objective, Scoreboard};

use super::view::SIDEBAR_TITLE;

const OBJECTIVE_NAME: &str = "towerwars";

#[derive(Debug)]
pub struct Hud {
    scoreboard: Scoreboard,
    objective: Objective,
    shown: Vec<String>,
}

impl Hud {
    pub fn new(scoreboard: Scoreboard) -> Self {
        let objective =
            scoreboard.register_objective(OBJECTIVE_NAME, &Criteria::dummy(), SIDEBAR_TITLE);
        scoreboard.set_display_slot(DisplaySlot::Sidebar, Some(&objective));
        Self {
            scoreboard,
            objective,
            shown: Vec::new(),
        }
    }

    pub fn render(&mut self, lines: Vec<String>) {
        if lines == self.shown {
            return;
        }
        for stale in self.shown.iter().filter(|line| !lines.contains(line)) {
            self.scoreboard.reset_scores(stale.as_str());
        }
        let top = i32::try_from(lines.len()).unwrap_or(i32::MAX);
        for (index, line) in lines.iter().enumerate() {
            let offset = i32::try_from(index).unwrap_or(i32::MAX);
            self.objective
                .score(line.as_str())
                .set(top.saturating_sub(offset));
        }
        self.shown = lines;
    }

    pub fn close(&self) {
        self.scoreboard.set_display_slot(DisplaySlot::Sidebar, None);
        self.objective.unregister();
    }
}
