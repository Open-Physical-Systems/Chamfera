use super::*;
use chamfera_geom::Point3;
use chamfera_render::StandardView;

/// A UV sphere of `radius` about `c`, outward winding.
fn sphere(c: Point3, radius: f64, rings: u32, segments: u32) -> TriMesh {
    let mut m = TriMesh::default();
    for i in 0..=rings {
        let t = std::f64::consts::PI * f64::from(i) / f64::from(rings);
        for j in 0..segments {
            let p = std::f64::consts::TAU * f64::from(j) / f64::from(segments);
            m.positions.push(Point3::new(
                c.x + radius * t.sin() * p.cos(),
                c.y + radius * t.sin() * p.sin(),
                c.z + radius * t.cos(),
            ));
        }
    }
    for i in 0..rings {
        for j in 0..segments {
            let a = i * segments + j;
            let b = i * segments + (j + 1) % segments;
            let (c2, d) = (a + segments, b + segments);
            if i > 0 {
                m.triangles.push([a, c2, b]);
            }
            if i + 1 < rings {
                m.triangles.push([b, c2, d]);
            }
        }
    }
    m
}

fn camera_at_origin(distance: f64) -> Camera {
    let mut c = Camera {
        target: Point3::ORIGIN,
        distance,
        ..Camera::default()
    };
    c.set_view(StandardView::Front);
    c
}

fn lambert(albedo: f32) -> Material {
    // An index of refraction of 1 has no reflecting interface: pure Lambert.
    Material {
        base: [albedo; 3],
        metallic: 0.0,
        roughness: 1.0,
        transmission: 0.0,
        ior: 1.0,
    }
}

#[test]
fn a_white_lambert_sphere_in_a_uniform_sky_shows_its_albedo() {
    // The furnace test. A convex body under a uniform sky of radiance L sees only the
    // sky, so one bounce off a Lambert surface of albedo rho gives exactly
    // L_o = int (rho / pi) L cos dw = rho L at every point; the background is L itself.
    let mesh = sphere(Point3::ORIGIN, 10.0, 48, 96);
    let bodies = [Body {
        mesh: &mesh,
        material: lambert(0.8),
    }];
    let cam = camera_at_origin(60.0);
    let r = Render::new(
        &bodies,
        &cam,
        Environment::uniform(1.0),
        Settings::new(32, 32, 16),
    )
    .unwrap();
    while !r.done() {
        r.step();
    }
    assert_eq!(r.samples(), 16);
    let img = r.radiance();
    // Pixels whose whole square is on the sphere: the sphere's radius on screen is
    // 10 / (60 tan(17.5 deg)) = 0.528 of the half height, 8.45 px; inside 6 px of the
    // centre (16, 16) every sample hits it.
    let (mut sphere_sum, mut sphere_n) = (0.0, 0);
    for y in 0..32 {
        for x in 0..32 {
            let p = img[y * 32 + x];
            let (dx, dy) = (x as f64 + 0.5 - 16.0, y as f64 + 0.5 - 16.0);
            let r2 = (dx * dx + dy * dy).sqrt();
            if r2 < 6.0 {
                for c in p {
                    assert!((c - 0.8).abs() < 0.03, "({x}, {y}): {p:?}");
                }
                sphere_sum += f64::from(p[0]);
                sphere_n += 1;
            } else if r2 > 11.0 {
                for c in p {
                    assert!((c - 1.0).abs() < 1e-6, "({x}, {y}): {p:?}");
                }
            }
        }
    }
    assert!(sphere_n > 50);
    assert!((sphere_sum / f64::from(sphere_n) - 0.8).abs() < 0.01);
}

#[test]
fn a_white_metal_sphere_in_a_uniform_sky_never_glows() {
    // Energy conservation in the renderer: F0 = 1 absorbs nothing, but single-scattering
    // microfacets lose some energy, so every sphere pixel is at most the sky's 1.
    let mesh = sphere(Point3::ORIGIN, 10.0, 32, 64);
    for roughness in [0.2, 0.7] {
        let bodies = [Body {
            mesh: &mesh,
            material: Material {
                base: [1.0; 3],
                metallic: 1.0,
                roughness,
                transmission: 0.0,
                ior: 1.5,
            },
        }];
        let r = Render::new(
            &bodies,
            &camera_at_origin(60.0),
            Environment::uniform(1.0),
            Settings::new(24, 24, 32),
        )
        .unwrap();
        while !r.done() {
            r.step();
        }
        let img = r.radiance();
        let mut mean = 0.0;
        for y in 9..15 {
            for x in 9..15 {
                let p = img[y * 24 + x];
                mean += f64::from(p[0]) / 36.0;
            }
        }
        assert!(mean <= 1.0 + 0.02, "{roughness}: {mean}");
        assert!(mean > 0.6, "{roughness}: {mean}");
    }
}

#[test]
fn the_bvh_finds_the_same_hits_as_testing_every_triangle() {
    // 2000 random triangles and 3000 random rays from a fixed seed.
    let mut rng = Rng::new(7, 3);
    let mut r = |lo: f64, hi: f64| lo + (hi - lo) * rng.f();
    let mut tris = Vec::new();
    while tris.len() < 2000 {
        let c = v3(r(-50.0, 50.0), r(-50.0, 50.0), r(-50.0, 50.0));
        let p =
            |r: &mut dyn FnMut(f64, f64) -> f64| c + v3(r(-4.0, 4.0), r(-4.0, 4.0), r(-4.0, 4.0));
        let (a, b, cc) = (p(&mut r), p(&mut r), p(&mut r));
        let n = (b - a).cross(cc - a).normalized().unwrap_or(V3::ONE);
        if let Some(t) = Tri::new(a, b, cc, [n; 3], 0) {
            tris.push(t);
        }
    }
    let bvh = Bvh::build(tris);
    let mut hits = 0;
    for _ in 0..3000 {
        let o = v3(r(-80.0, 80.0), r(-80.0, 80.0), r(-80.0, 80.0));
        let Some(d) = v3(r(-1.0, 1.0), r(-1.0, 1.0), r(-1.0, 1.0)).normalized() else {
            continue;
        };
        let a = bvh.intersect(o, d, 0.0, f64::INFINITY);
        let b = bvh.brute_force(o, d, 0.0, f64::INFINITY);
        match (a, b) {
            (Some(a), Some(b)) => {
                hits += 1;
                assert!((a.t - b.t).abs() < 1e-9, "{} {}", a.t, b.t);
                assert!(bvh.occluded(o, d, 0.0, f64::INFINITY));
            }
            (None, None) => assert!(!bvh.occluded(o, d, 0.0, f64::INFINITY)),
            (a, b) => panic!("bvh {a:?}, brute force {b:?}"),
        }
    }
    assert!(hits > 100, "{hits}");
}

#[test]
fn sizes_and_sample_counts_out_of_range_are_refused() {
    let mesh = sphere(Point3::ORIGIN, 1.0, 4, 8);
    let bodies = [Body {
        mesh: &mesh,
        material: lambert(0.5),
    }];
    let cam = Camera::default();
    for (w, h, s) in [
        (0, 10, 1),
        (10, 0, 1),
        (10, 10, 0),
        (MAX_SIZE + 1, 10, 1),
        (10, MAX_SIZE + 1, 1),
        (10, 10, MAX_SAMPLES + 1),
    ] {
        let e = Render::new(
            &bodies,
            &cam,
            Environment::uniform(1.0),
            Settings::new(w, h, s),
        );
        assert!(
            matches!(e, Err(RenderError::OutOfRange { .. })),
            "{w} {h} {s}"
        );
    }
}

#[test]
fn hostile_meshes_and_cameras_never_panic() {
    let mut bad = sphere(Point3::ORIGIN, 1.0, 4, 8);
    if let Some(p) = bad.positions.get_mut(3) {
        p.x = f64::NAN;
    }
    bad.triangles.push([0, 0, 0]);
    bad.triangles.push([1, 2, 99_999]);
    let empty = TriMesh::default();
    let bodies = [
        Body {
            mesh: &bad,
            material: Material {
                base: [f32::NAN; 3],
                metallic: f32::INFINITY,
                roughness: -3.0,
                transmission: 0.7,
                ior: 0.0,
            },
        },
        Body {
            mesh: &empty,
            material: lambert(0.5),
        },
    ];
    let cam = Camera {
        distance: f64::NAN,
        yaw: f64::INFINITY,
        ..Camera::default()
    };
    let env = Environment::preset(
        chamfera_render::EnvironmentPreset::Studio,
        &Camera::default(),
    );
    let r = Render::new(&bodies, &cam, env, Settings::new(8, 8, 2)).unwrap();
    r.step();
    r.step();
    r.step();
    assert_eq!(r.samples(), 2);
    assert_eq!(r.rgba().len(), 8 * 8 * 4);
    // Nothing at all to render still gives a picture of the environment.
    let r = Render::new(
        &[],
        &Camera::default(),
        Environment::uniform(0.5),
        Settings::new(4, 4, 1),
    )
    .unwrap();
    r.step();
    assert!(r.radiance().iter().all(|p| (p[0] - 0.5).abs() < 1e-6));
}

#[test]
fn the_picture_does_not_depend_on_the_thread_count_and_the_png_has_its_size() {
    let mesh = sphere(Point3::ORIGIN, 10.0, 12, 24);
    let bodies = [Body {
        mesh: &mesh,
        material: chamfera_render::material::by_id("brass")
            .unwrap()
            .material(),
    }];
    let env = Environment::preset(
        chamfera_render::EnvironmentPreset::Studio,
        &camera_at_origin(60.0),
    );
    let run = |threads: usize| {
        let mut s = Settings::new(20, 12, 3);
        s.threads = threads;
        let r = Render::new(&bodies, &camera_at_origin(60.0), env.clone(), s).unwrap();
        while !r.done() {
            r.step();
        }
        r.rgba()
    };
    assert_eq!(run(1), run(4));
    let r = Render::new(
        &bodies,
        &camera_at_origin(60.0),
        env.clone(),
        Settings::new(20, 12, 1),
    )
    .unwrap();
    r.step();
    let png = r.png().unwrap();
    // PNG signature, then IHDR: width and height as big-endian u32 at bytes 16 and 20.
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    assert_eq!(u32::from_be_bytes([png[16], png[17], png[18], png[19]]), 20);
    assert_eq!(u32::from_be_bytes([png[20], png[21], png[22], png[23]]), 12);
    // Cancelling stops it.
    r.cancel();
    assert!(r.done());
}

#[test]
fn a_body_casts_a_shadow_and_glass_casts_a_lighter_one() {
    // A black sky and one small light towards (1, 0, 1) / sqrt 2, over a white pad
    // (albedo 0.8, top at z = 0.1) with an opaque cube and a glass cube floating at
    // z = 10..20, seen from straight above in orthographic.
    let mut env = Environment::uniform(0.0);
    env.floor = None;
    env.lights = chamfera_render::environment::Light::from_irradiance(
        chamfera_geom::Vec3::new(1.0, 0.0, 1.0),
        [3.0; 3],
        0.02,
    )
    .into_iter()
    .collect();
    let pad = TriMesh::cuboid(Point3::new(-30.0, -30.0, 0.0), Point3::new(30.0, 30.0, 0.1));
    let cube = TriMesh::cuboid(Point3::new(-5.0, 10.0, 10.0), Point3::new(5.0, 20.0, 20.0));
    let glass_cube = TriMesh::cuboid(
        Point3::new(-5.0, -20.0, 10.0),
        Point3::new(5.0, -10.0, 20.0),
    );
    let glass = chamfera_render::material::by_id("glass")
        .unwrap()
        .material();
    let bodies = [
        Body {
            mesh: &pad,
            material: lambert(0.8),
        },
        // Black, so it sends no light from the bright pad back into its own shadow.
        Body {
            mesh: &cube,
            material: lambert(0.0),
        },
        Body {
            mesh: &glass_cube,
            material: glass,
        },
    ];
    let mut cam = Camera {
        target: Point3::ORIGIN,
        distance: 200.0,
        projection: Projection::Orthographic,
        ..Camera::default()
    };
    cam.set_view(StandardView::Top);
    let r = Render::new(&bodies, &cam, env, Settings::new(64, 64, 8)).unwrap();
    while !r.done() {
        r.step();
    }
    let img = r.radiance();
    // From the top, +X is right and +Y up on screen; the half height is
    // 200 tan(17.5 deg) = 63.06 mm over 32 px.
    let px = |x: f64, y: f64| {
        let k = 32.0 / (200.0 * 17.5f64.to_radians().tan());
        let (i, j) = ((32.0 + x * k) as usize, (32.0 - y * k) as usize);
        img[j * 64 + i]
    };
    // Lit pad: E = 3 cos 45 = 2.1213, radiance rho / pi E = 0.8 / pi * 2.1213 = 0.5402.
    let lit = px(20.0, 0.0)[0];
    assert!((lit - 0.5402).abs() < 0.01, "{lit}");
    // (-15, 15): the ray (x + s, y, 0.1 + s) to the light is inside the cube for s in
    // 10..19.9, so the point is in full shadow, the sky is black and the black cube
    // reflects nothing; only light bent through the glass cube could reach it.
    let dark = px(-15.0, 15.0)[0];
    assert!(dark.abs() < 0.01, "{dark}");
    // (-15, -15): the same ray goes through the glass cube, in through x = -5 and out
    // through z = 20, both at 45 degrees. Each surface passes 1 - F with F = 0.05025 at
    // 45 degrees for n = 1.5 (rs^2 = 0.09202, rp^2 = 0.008472), and the two crossings
    // pass the glass's colour once: red 214 is 0.6724 linear. So
    // 0.5402 * 0.6724 * 0.94975^2 = 0.3277.
    let tinted = px(-15.0, -15.0)[0];
    assert!((tinted - 0.3277).abs() < 0.01, "{tinted}");
}
