//! Retail camera field of view from `[Display] fDefaultWorldFOV` and `fDefault1stPersonFOV`.
//!
//! Retail keeps both settings as horizontal angles in degrees. `PlayerCamera` copies them at
//! construction (`0x1408f79f0`), and `0x1408fb1e0` returns the first-person value while the
//! first-person state is current and the world value otherwise (RF-CAM-009 in
//! `docs/research/retail/registry.md`, traced in `docs/research/retail/q7a-camera-states.md`).
//! Mudcrab maps WALK, its first-person camera, to the first-person value and NOCLIP, its free
//! camera, to the world value.
//!
//! Bevy's `PerspectiveProjection::fov` is vertical. The horizontal angle is held for the live
//! window aspect, the same convention as the `hfov` of a `--shots` pose. Whether retail holds the
//! horizontal angle at the window's aspect or at a fixed reference aspect is not yet measured
//! against a retail capture; [`vertical_fov_radians`] is the one place that decides it.

use crate::{config::EngineConfig, physics::MoveMode, settings::RenderingSettings};
use bevy::{prelude::*, window::PrimaryWindow};

/// The executable default of both `fDefaultWorldFOV` and `fDefault1stPersonFOV` (RF-CAM-009).
pub const RETAIL_DEFAULT_FOV_DEGREES: f32 = 80.0;

/// Smallest horizontal angle the settings menu offers and an INI value may set. Mudcrab's bound,
/// not a retail one: retail's own limits are not traced.
pub const MIN_FOV_DEGREES: f32 = 30.0;
/// Largest horizontal angle the settings menu offers and an INI value may set. Mudcrab's bound,
/// not a retail one.
pub const MAX_FOV_DEGREES: f32 = 150.0;

/// Whether `degrees` is a horizontal field of view the engine accepts.
pub fn valid_fov_degrees(degrees: f32) -> bool {
    degrees.is_finite() && (MIN_FOV_DEGREES..=MAX_FOV_DEGREES).contains(&degrees)
}

/// Bevy's vertical field of view, in radians, for a horizontal angle held at `aspect` (width over
/// height). A degenerate aspect leaves the angle unconverted.
pub fn vertical_fov_radians(horizontal_degrees: f32, aspect: f32) -> f32 {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    crate::shots::vertical_fov_degrees(horizontal_degrees, aspect).to_radians()
}

/// The horizontal angle retail uses for the current camera mode.
pub fn horizontal_fov_degrees(settings: &RenderingSettings, mode: Option<MoveMode>) -> f32 {
    match mode {
        Some(MoveMode::Walk) => settings.first_person_fov_degrees,
        Some(MoveMode::Noclip) | None => settings.world_fov_degrees,
    }
}

/// Applies the two `[Display]` angles an INI set, warning about and ignoring unusable values.
pub(crate) fn apply_ini_value(config: &mut EngineConfig, key: &str, raw: &str) {
    match raw.parse::<f32>() {
        Ok(value) if valid_fov_degrees(value) => {
            match key {
                "fDefaultWorldFOV" => config.world_fov_degrees = value,
                _ => config.first_person_fov_degrees = value,
            }
            config.settings_overrides.push(format!("ini:{key}"));
        }
        _ => eprintln!(
            "warning: ignoring {key}={raw:?}; expected a horizontal angle from {MIN_FOV_DEGREES} to {MAX_FOV_DEGREES} degrees"
        ),
    }
}

/// Keeps every world camera's vertical angle matched to the mode's horizontal angle, the window's
/// shape and the menu. Runs every frame because a resize or a WALK/NOCLIP toggle changes the
/// result without a preference change; it writes only on a real difference.
pub(crate) fn follow_camera_fov(
    settings: Res<RenderingSettings>,
    mode: Option<Res<MoveMode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cameras: Query<&mut Projection, With<crate::world::components::StreamingCamera>>,
) {
    let horizontal = horizontal_fov_degrees(&settings, mode.as_deref().copied());
    // silent-ok: a run without one primary window falls back to the projection's own aspect
    let window_aspect = windows
        .single()
        .ok()
        .map(|window| window.width() / window.height());
    for mut projection in &mut cameras {
        let Projection::Perspective(perspective) = projection.as_ref() else {
            continue;
        };
        let aspect = window_aspect.unwrap_or(perspective.aspect_ratio);
        let fov = vertical_fov_radians(horizontal, aspect);
        if perspective.fov != fov
            && let Projection::Perspective(perspective) = projection.as_mut()
        {
            perspective.fov = fov;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertical_degrees(horizontal: f32, aspect: f32) -> f32 {
        vertical_fov_radians(horizontal, aspect).to_degrees()
    }

    /// The retail default at the common window shapes: 2 atan(tan(40°) / aspect).
    #[test]
    fn retail_default_converts_at_16_9_16_10_and_21_9() {
        for (aspect, expected) in [
            (16.0 / 9.0, 50.534_0),
            (16.0 / 10.0, 55.348_4),
            (21.0 / 9.0, 39.558_6),
        ] {
            let vertical = vertical_degrees(RETAIL_DEFAULT_FOV_DEGREES, aspect);
            assert!(
                (vertical - expected).abs() < 1.0e-3,
                "aspect {aspect}: {vertical}"
            );
        }
    }

    /// Holding the horizontal angle means a wider window is shorter vertically, and a square
    /// window keeps the angle as it is.
    #[test]
    fn the_horizontal_angle_is_held_across_window_shapes() {
        assert!((vertical_degrees(80.0, 1.0) - 80.0).abs() < 1.0e-4);
        assert!(vertical_degrees(80.0, 21.0 / 9.0) < vertical_degrees(80.0, 16.0 / 9.0));
        for aspect in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!((vertical_degrees(80.0, aspect) - 80.0).abs() < 1.0e-4);
        }
    }

    #[test]
    fn walk_uses_the_first_person_angle_and_noclip_the_world_angle() {
        let settings = RenderingSettings {
            world_fov_degrees: 90.0,
            first_person_fov_degrees: 70.0,
            ..Default::default()
        };
        assert_eq!(
            horizontal_fov_degrees(&settings, Some(MoveMode::Walk)),
            70.0
        );
        assert_eq!(
            horizontal_fov_degrees(&settings, Some(MoveMode::Noclip)),
            90.0
        );
        assert_eq!(horizontal_fov_degrees(&settings, None), 90.0);
    }

    #[test]
    fn follow_camera_fov_tracks_the_window_shape_and_the_mode() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RenderingSettings {
                world_fov_degrees: 90.0,
                first_person_fov_degrees: 80.0,
                ..Default::default()
            })
            .insert_resource(MoveMode::Walk)
            .add_systems(Update, follow_camera_fov);
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: (1920, 1080).into(),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = app
            .world_mut()
            .spawn((
                Projection::Perspective(PerspectiveProjection::default()),
                crate::world::components::StreamingCamera,
            ))
            .id();
        let fov = |app: &App| match app.world().get::<Projection>(camera).unwrap() {
            Projection::Perspective(perspective) => perspective.fov,
            _ => panic!("perspective camera"),
        };
        app.update();
        assert!((fov(&app) - vertical_fov_radians(80.0, 16.0 / 9.0)).abs() < 1.0e-6);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .resolution
            .set(2520.0, 1080.0);
        app.update();
        assert!((fov(&app) - vertical_fov_radians(80.0, 21.0 / 9.0)).abs() < 1.0e-6);
        app.insert_resource(MoveMode::Noclip);
        app.update();
        assert!((fov(&app) - vertical_fov_radians(90.0, 21.0 / 9.0)).abs() < 1.0e-6);
    }
}
