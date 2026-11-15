//! Build steps in the window: which page of the PDF a slide shows at a step,
//! what GO brings next (the next build or the next slide), and going
//! through the steps.

use super::*;

impl App {
    /// Build steps of a slide (0: none).
    pub(super) fn steps_of(&self, slide: usize) -> usize {
        self.slides.get(slide).map_or(0, |s| s.builds.len())
    }

    /// The page (in the PDF) of a slide at a step.
    pub(super) fn page_at(&self, slide: usize, step: usize) -> usize {
        self.first_page.get(slide).copied().unwrap_or(slide) + step.min(self.steps_of(slide))
    }

    /// The finished slide (for the list).
    pub(super) fn full_page(&self, slide: usize) -> usize {
        self.page_at(slide, self.steps_of(slide))
    }

    pub(super) fn live_page(&self) -> Option<usize> {
        self.player
            .current
            .map(|c| self.page_at(c, self.player.step))
    }

    pub(super) fn prev_page(&self) -> Option<usize> {
        self.player
            .previous
            .map(|p| self.page_at(p, self.player.prev_step))
    }

    /// What GO brings: the next build of this slide, else the next slide.
    pub(super) fn next_cue(&self) -> Option<(usize, usize)> {
        if let Some(c) = self.player.current {
            if self.player.step < self.steps_of(c) {
                return Some((c, self.player.step + 1));
            }
        }
        self.next_index().map(|n| (n, 0))
    }

    pub(super) fn cue_page(&self, cue: Option<(usize, usize)>) -> Option<usize> {
        cue.map(|(s, k)| self.page_at(s, k))
    }

    /// "Slide 4", or "Build 2/3" for a step of the slide on air.
    pub(super) fn cue_label(&self, (slide, step): (usize, usize)) -> String {
        if step == 0 {
            format!("Slide {}", slide + 1)
        } else {
            format!("Build {step}/{}", self.steps_of(slide))
        }
    }

    /// The pages of the slides: where each one's first step is.
    pub(super) fn set_pages(&mut self, pages: usize) {
        let total: usize = self.slides.iter().map(|s| s.builds.len() + 1).sum();
        if total != pages {
            // Not the expanded copy (no LibreOffice copy, an older cache):
            // a page per slide, no builds.
            for s in &mut self.slides {
                s.builds.clear();
            }
        }
        let mut at = 0;
        self.first_page = self
            .slides
            .iter()
            .map(|s| {
                let p = at;
                at += s.builds.len() + 1;
                p
            })
            .collect();
    }

    /// Go to a build step of the slide on air.
    pub(super) fn build_step(&mut self, step: usize, why: logui::Why) {
        let Some(c) = self.player.current else { return };
        let step = step.min(self.steps_of(c));
        let fade = if step > self.player.step {
            self.slides
                .get(c)
                .and_then(|s| s.builds.get(step - 1))
                .copied()
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let t = if fade > 0.0 {
            Transition::Fade(fade)
        } else {
            Transition::Cut
        };
        self.log.why = Some(why);
        self.player.show_at(Some(c), step, t);
    }

    /// A slide at a step (GO− to the slide before shows it finished).
    pub(super) fn goto_step(&mut self, slide: usize, step: usize, why: logui::Why) {
        if slide >= self.slides.len() {
            return;
        }
        let t = self.transition_of(slide);
        self.log.why = Some(why);
        self.player
            .show_at(Some(slide), step.min(self.steps_of(slide)), t);
        self.scroll_to_live = true;
    }
}
