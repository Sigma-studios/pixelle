//! The animation view: one of the open sheet's animations, a frame at a time, its neighbours
//! ghosted, with a strip of its steps below. What is painted is the sheet's frame itself.

use eframe::egui::{self, Color32, Painter, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use pixelle_core::IRect;
use pixelle_core::folder;

use crate::app::{Editor, Selected};
use crate::view::View;
use crate::workspace;

/// An animation open in the view.
#[derive(Clone)]
pub struct AnimView {
    pub anim: String,
    pub step: usize,
    /// Playing since this time on the editor's clock.
    pub playing: Option<f32>,
    /// The frame shown, so the view can follow it to the next one.
    pub rect: IRect,
    /// How the sheet was being looked at, to go back to.
    pub back: View,
}

/// A step of an animation on a sprite canvas.
#[derive(Clone, Copy)]
pub struct Step {
    /// The sheet's frame it shows.
    pub frame: usize,
    pub rect: IRect,
    pub duration: f32,
}

impl Editor {
    /// The sheet of the open sprite canvas and the steps of one of its animations.
    pub fn anim_steps(&self, anim: &str) -> Option<(String, Vec<Step>)> {
        let p = &self.doc.project;
        let sheet = p.sprites.iter().find(|s| !workspace::is_anim(s))?;
        let a = p.sprite(&format!("{}{}{anim}", sheet.name, folder::ANIM_SEPARATOR))?;
        let steps = a
            .frames
            .iter()
            .filter_map(|f| {
                let frame = sheet.frames.iter().position(|sf| sf.rect == f.rect)?;
                Some(Step { frame, rect: f.rect, duration: f.duration.max(0.001) })
            })
            .collect();
        Some((sheet.name.clone(), steps))
    }

    pub fn open_anim(&mut self, anim: &str) {
        let Some((_, steps)) = self.anim_steps(anim).filter(|(_, s)| !s.is_empty()) else { return };
        self.commit_floating();
        self.selection = None;
        self.placing_point = None;
        let rect = steps[0].rect;
        self.anim = Some(AnimView { anim: anim.to_string(), step: 0, playing: None, rect, back: self.view });
        self.pending_fit_rect = Some(rect.grow(2));
        self.fit_pending = true;
        self.sync_anim();
    }

    pub fn close_anim(&mut self) {
        if let Some(a) = self.anim.take() {
            self.view = a.back;
            self.doc.only = None;
            self.drag = None;
        }
    }

    pub fn step_anim(&mut self, by: i32) {
        let Some(a) = &self.anim else { return };
        let n = self.anim_steps(&a.anim).map_or(1, |(_, s)| s.len().max(1)) as i32;
        let a = self.anim.as_mut().expect("checked");
        a.playing = None;
        a.step = (a.step as i32 + by).rem_euclid(n) as usize;
    }

    pub fn toggle_play(&mut self) {
        let t = self.time();
        let Some(a) = &self.anim else { return };
        let Some((_, steps)) = self.anim_steps(&a.anim) else { return };
        let a = self.anim.as_mut().expect("checked");
        a.playing = match a.playing {
            Some(_) => None,
            // From the step shown.
            None => Some(t - steps[..a.step.min(steps.len())].iter().map(|s| s.duration).sum::<f32>()),
        };
    }

    /// Keep the view on the step shown: after a step, a frame of the animation edited or
    /// removed, or an undo. Painting is kept to that frame.
    pub fn sync_anim(&mut self) {
        let Some(a) = &self.anim else { return };
        let Some((sheet, steps)) = self.anim_steps(&a.anim).filter(|(_, s)| !s.is_empty()) else {
            self.close_anim();
            return;
        };
        let t = self.time();
        let a = self.anim.as_mut().expect("checked");
        if let Some(start) = a.playing {
            let total: f32 = steps.iter().map(|s| s.duration).sum();
            let mut clock = (t - start).rem_euclid(total);
            a.step = steps.len() - 1;
            for (i, s) in steps.iter().enumerate() {
                if clock < s.duration {
                    a.step = i;
                    break;
                }
                clock -= s.duration;
            }
        }
        a.step = a.step.min(steps.len() - 1);
        let step = steps[a.step];
        if step.rect != a.rect {
            let d = step.rect.min() - a.rect.min();
            self.view.center += vec2(d.x as f32, d.y as f32);
            a.rect = step.rect;
        }
        self.doc.only = Some(step.rect);
        self.selected = Selected::Sprite { name: sheet, frame: step.frame };
    }

    /// The neighbouring steps' frames, faint, under the one being drawn.
    pub fn paint_ghosts(&mut self, ctx: &egui::Context, painter: &Painter, screen: Rect) {
        let Some(a) = self.anim.clone() else { return };
        let Some((sheet, steps)) = self.anim_steps(&a.anim) else { return };
        let r = self.view.rect_to_screen(screen, a.rect);
        painter.rect_filled(r, 0.0, Color32::from_gray(52));
        if !self.opts.ghosts || a.playing.is_some() || steps.len() < 2 {
            return;
        }
        let n = steps.len() as i32;
        let current = steps[a.step].frame;
        let mut drawn = vec![a.step];
        // Farthest first, so the nearest ones are on top.
        for k in (1..=self.opts.ghost_count as i32).rev() {
            for (dir, tint) in [(-1, [255, 110, 110]), (1, [110, 170, 255])] {
                let i = (a.step as i32 + dir * k).rem_euclid(n) as usize;
                if drawn.contains(&i) || steps[i].frame == current {
                    continue;
                }
                drawn.push(i);
                let Some((tex, size)) = self.backend.frame(ctx, &self.doc.project, &sheet, steps[i].frame) else { continue };
                let alpha = (110.0 / k as f32) as u8;
                let [r0, g0, b0] = if self.opts.ghost_tint { tint } else { [255, 255, 255] };
                let at = IRect::new(a.rect.x, a.rect.y, size[0], size[1]);
                let tint = Color32::from_rgba_unmultiplied(r0, g0, b0, alpha);
                painter.image(tex, self.view.rect_to_screen(screen, at), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
            }
        }
    }

    /// Controls for the animation open, and its steps as a strip to pick from.
    pub fn anim_bar(&mut self, ui: &mut egui::Ui) {
        let Some(a) = self.anim.clone() else { return };
        let Some((sheet, steps)) = self.anim_steps(&a.anim) else { return };
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("◀ Sheet").on_hover_text("Back to the whole sheet (Esc)").clicked() {
                self.close_anim();
            }
            ui.label(RichText::new(&a.anim).strong());
            ui.separator();
            if ui.button("◀").on_hover_text("Previous step  ← [").clicked() {
                self.step_anim(-1);
            }
            let play = if a.playing.is_some() { "Pause" } else { "Play" };
            if ui.button(play).on_hover_text("Enter").clicked() {
                self.toggle_play();
            }
            if ui.button("▶").on_hover_text("Next step  → ]").clicked() {
                self.step_anim(1);
            }
            ui.label(format!("step {} of {}", a.step + 1, steps.len()));
            let step = steps[a.step];
            let mut ms = (step.duration * 1000.0).round() as u32;
            if ui.add(egui::DragValue::new(&mut ms).suffix(" ms").range(1..=10_000)).changed() {
                let (anim, si) = (a.anim.clone(), a.step);
                self.edit_sprite("Edit step", Some(&format!("step-{anim}-{si}")), |s| {
                    if let Some(st) = s.anims.get_mut(&anim).and_then(|v| v.get_mut(si)) {
                        st.duration = ms as f32 / 1000.0;
                    }
                });
            }
            ui.separator();
            ui.checkbox(&mut self.opts.ghosts, "Ghosts").on_hover_text("Show the steps before and after, faintly");
            ui.add_enabled(self.opts.ghosts, egui::DragValue::new(&mut self.opts.ghost_count).range(1..=3))
                .on_hover_text("How many on each side");
            ui.add_enabled(self.opts.ghosts, egui::Checkbox::new(&mut self.opts.ghost_tint, "Tint"))
                .on_hover_text("Red before, blue after");
            ui.separator();
            if ui.button("+ New frame").on_hover_text("A copy of this frame, added to the end of the sheet, as the next step").clicked() {
                self.new_anim_frame(&a, step);
            }
            if ui.add_enabled(steps.len() > 1, egui::Button::new("Remove step")).clicked() {
                let (anim, si) = (a.anim.clone(), a.step);
                self.edit_sprite("Remove step", None, |s| {
                    s.anims.get_mut(&anim).expect("the animation").remove(si);
                });
            }
        });

        // The steps.
        let mut pick = None;
        egui::ScrollArea::horizontal().id_salt("anim-steps").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, s) in steps.iter().enumerate() {
                    let current = i == a.step;
                    let tex = self.backend.frame(ui.ctx(), &self.doc.project, &sheet, s.frame);
                    let (w, h) = (s.rect.w.max(1) as f32, s.rect.h.max(1) as f32);
                    let k = (40.0 / w.max(h)).max(1.0).floor();
                    let size = vec2((w * k).max(28.0), h * k);
                    let r = ui
                        .vertical(|ui| {
                            let (r, resp) = ui.allocate_exact_size(size, Sense::click());
                            let p = ui.painter();
                            p.rect_filled(r, 2.0, Color32::from_gray(if current { 70 } else { 45 }));
                            if let Some((tex, _)) = tex {
                                let img = Rect::from_center_size(r.center(), vec2(w * k, h * k));
                                p.image(tex, img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            let stroke = if current { Stroke::new(2.0, Color32::from_rgb(255, 210, 60)) } else { Stroke::new(1.0, Color32::from_white_alpha(30)) };
                            p.rect_stroke(r, 2.0, stroke, StrokeKind::Outside);
                            ui.label(RichText::new(format!("{} · {:.0}", s.frame, s.duration * 1000.0)).small().weak());
                            resp
                        })
                        .inner;
                    if r.on_hover_text(format!("Step {}: frame {}, {:.0} ms", i + 1, s.frame, s.duration * 1000.0)).clicked() {
                        pick = Some(i);
                    }
                }
            });
        });
        if let Some(i) = pick
            && let Some(a) = &mut self.anim
        {
            a.playing = None;
            a.step = i;
        }
        if a.playing.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    /// Copy the frame of the step shown to a new frame at the end of the sheet, so the indices
    /// the game uses for the others stay, and make it the next step.
    pub fn new_anim_frame(&mut self, a: &AnimView, step: Step) {
        let (anim, si) = (a.anim.clone(), a.step);
        self.edit_sprite("New frame", None, |s| {
            let mut f = s.frames[step.frame].clone();
            f.name = None;
            let at = s.frames.len();
            s.insert_frame(at, f);
            let steps = s.anims.get_mut(&anim).expect("the animation");
            steps.insert(si + 1, folder::Step { frame: at, duration: step.duration });
        });
        if let Some(a) = &mut self.anim {
            a.step = si + 1;
            a.playing = None;
        }
    }
}
