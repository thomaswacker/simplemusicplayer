//! Vector icons painted with egui shapes (crisp at any scale, no font dependency).

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use std::f32::consts::PI;

#[derive(Debug, Clone, Copy)]
pub enum Icon {
    Play,
    Pause,
    Previous,
    Next,
    VolumeHigh,
    VolumeLow,
    VolumeMuted,
    Home,
    Up,
    Refresh,
    Folder,
    FolderOpen,
    Note,
}

/// Paints `icon` into `rect` (treated as a square around its center).
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    // Map unit coordinates (-0.5..0.5) to the icon square.
    let at = |x: f32, y: f32| c + Vec2::new(x * s, y * s);
    let stroke = Stroke::new((s * 0.1).max(1.5), color);
    let fill = |pts: Vec<Pos2>| Shape::convex_polygon(pts, color, Stroke::NONE);

    match icon {
        Icon::Play => {
            p.add(fill(vec![at(-0.3, -0.45), at(0.45, 0.0), at(-0.3, 0.45)]));
        }
        Icon::Pause => {
            let w = 0.24;
            for x in [-0.32, 0.08] {
                p.rect_filled(
                    Rect::from_min_max(at(x, -0.42), at(x + w, 0.42)),
                    s * 0.05,
                    color,
                );
            }
        }
        Icon::Next => {
            p.add(fill(vec![at(-0.4, -0.4), at(0.22, 0.0), at(-0.4, 0.4)]));
            p.rect_filled(
                Rect::from_min_max(at(0.24, -0.4), at(0.4, 0.4)),
                s * 0.03,
                color,
            );
        }
        Icon::Previous => {
            p.add(fill(vec![at(0.4, -0.4), at(-0.22, 0.0), at(0.4, 0.4)]));
            p.rect_filled(
                Rect::from_min_max(at(-0.4, -0.4), at(-0.24, 0.4)),
                s * 0.03,
                color,
            );
        }
        Icon::VolumeHigh | Icon::VolumeLow | Icon::VolumeMuted => {
            // Speaker body
            p.add(fill(vec![
                at(-0.48, -0.16),
                at(-0.26, -0.16),
                at(0.0, -0.42),
                at(0.0, 0.42),
                at(-0.26, 0.16),
                at(-0.48, 0.16),
            ]));
            let thin = Stroke::new((s * 0.08).max(1.4), color);
            match icon {
                Icon::VolumeMuted => {
                    p.line_segment([at(0.14, -0.16), at(0.46, 0.16)], thin);
                    p.line_segment([at(0.14, 0.16), at(0.46, -0.16)], thin);
                }
                _ => {
                    p.add(arc(at(0.0, 0.0), s * 0.2, -PI / 4.0, PI / 4.0, thin));
                    if matches!(icon, Icon::VolumeHigh) {
                        p.add(arc(at(0.0, 0.0), s * 0.4, -PI / 4.0, PI / 4.0, thin));
                    }
                }
            }
        }
        Icon::Home => {
            p.add(Shape::line(
                vec![at(-0.42, 0.0), at(0.0, -0.4), at(0.42, 0.0)],
                stroke,
            ));
            p.add(Shape::line(
                vec![
                    at(-0.28, -0.1),
                    at(-0.28, 0.42),
                    at(0.28, 0.42),
                    at(0.28, -0.1),
                ],
                stroke,
            ));
            p.rect_filled(
                Rect::from_min_max(at(-0.08, 0.14), at(0.08, 0.42)),
                0.0,
                color,
            );
        }
        Icon::Up => {
            p.line_segment([at(0.0, 0.42), at(0.0, -0.36)], stroke);
            p.add(Shape::line(
                vec![at(-0.32, -0.06), at(0.0, -0.4), at(0.32, -0.06)],
                stroke,
            ));
        }
        Icon::Refresh => {
            let r = s * 0.36;
            p.add(arc(c, r, -PI * 0.35, PI * 1.25, stroke));
            // Arrow head at the arc start (upper right).
            let a = -PI * 0.35;
            let tip = c + Vec2::angled(a) * r;
            let tangent = Vec2::angled(a - PI / 2.0); // direction of travel at the start
            let normal = Vec2::angled(a);
            let h = s * 0.2;
            p.add(fill(vec![
                tip + tangent * h * 0.9,
                tip + normal * h * 0.6,
                tip - normal * h * 0.6,
            ]));
        }
        Icon::Folder | Icon::FolderOpen => {
            let body = vec![
                at(-0.46, -0.32),
                at(-0.12, -0.32),
                at(-0.02, -0.2),
                at(0.46, -0.2),
                at(0.46, 0.36),
                at(-0.46, 0.36),
            ];
            if matches!(icon, Icon::Folder) {
                // Tab + body as two convex parts.
                p.add(fill(vec![
                    at(-0.46, -0.32),
                    at(-0.12, -0.32),
                    at(-0.02, -0.2),
                    at(-0.46, -0.2),
                ]));
                p.rect_filled(
                    Rect::from_min_max(at(-0.46, -0.22), at(0.46, 0.36)),
                    s * 0.05,
                    color,
                );
            } else {
                let mut outline = body;
                outline.push(at(-0.46, -0.32));
                p.add(Shape::line(
                    outline,
                    Stroke::new((s * 0.08).max(1.4), color),
                ));
                p.line_segment(
                    [at(-0.46, -0.06), at(0.46, -0.06)],
                    Stroke::new((s * 0.08).max(1.4), color),
                );
            }
        }
        Icon::Note => {
            p.circle_filled(at(-0.16, 0.26), s * 0.17, color);
            p.rect_filled(
                Rect::from_min_max(at(-0.02, -0.42), at(0.08, 0.26)),
                0.0,
                color,
            );
            p.add(fill(vec![
                at(0.08, -0.42),
                at(0.38, -0.24),
                at(0.38, -0.08),
                at(0.08, -0.24),
            ]));
        }
    }
}

fn arc(center: Pos2, radius: f32, from: f32, to: f32, stroke: Stroke) -> Shape {
    let steps = 24;
    let pts = (0..=steps)
        .map(|i| {
            let t = from + (to - from) * i as f32 / steps as f32;
            center + Vec2::angled(t) * radius
        })
        .collect::<Vec<_>>();
    Shape::line(pts, stroke)
}
