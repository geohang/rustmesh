//! Templates to start from: a few general geometries and the surveys of
//! several fields. Each has a handful of parameters ([`Example::params`]);
//! [`Example::build`] turns them into a plain [`Geometry2D`] that can then be
//! edited like any other.
//!
//! The geophysics templates follow pyGIMLi's markers: sensors are nodes with
//! marker -99, the ground surface edges -1, the other outer edges -2.

use std::f64::consts::PI;

use crate::model::{
    ellipse, points_along, rectangle, Domain, Geometry2D, Line, Point, Shape, Sizing, Zone,
};

/// pyGIMLi's node marker for sensors.
pub const SENSOR_MARKER: i32 = -99;
/// pyGIMLi's boundary marker for the ground surface (no flow).
pub const SURFACE_MARKER: i32 = -1;
/// pyGIMLi's boundary marker for the outer sides and bottom (mixed).
pub const OUTER_MARKER: i32 = -2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    General,
    Geophysics,
    Hydrology,
}

impl Field {
    pub const ALL: [Field; 3] = [Field::General, Field::Geophysics, Field::Hydrology];

    pub fn title(self) -> &'static str {
        match self {
            Field::General => "General",
            Field::Geophysics => "Geophysics",
            Field::Hydrology => "Hydrology",
        }
    }
}

/// One number a template takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Param {
    pub key: &'static str,
    pub label: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    /// A count: whole numbers only.
    pub whole: bool,
}

const fn num(key: &'static str, label: &'static str, default: f64, min: f64, max: f64) -> Param {
    Param {
        key,
        label,
        default,
        min,
        max,
        whole: false,
    }
}

const fn count(key: &'static str, label: &'static str, default: f64, min: f64, max: f64) -> Param {
    Param {
        key,
        label,
        default,
        min,
        max,
        whole: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Example {
    Rectangle,
    PlateWithHole,
    Channel,
    LShape,
    SurfaceErt,
    Crosshole,
    Refraction,
    GravityMagnetics,
    Magnetotelluric,
    AquiferWell,
    Watershed,
}

impl Example {
    pub const ALL: [Example; 11] = [
        Example::Rectangle,
        Example::PlateWithHole,
        Example::Channel,
        Example::LShape,
        Example::SurfaceErt,
        Example::Crosshole,
        Example::Refraction,
        Example::GravityMagnetics,
        Example::Magnetotelluric,
        Example::AquiferWell,
        Example::Watershed,
    ];

    /// Short name for command lines and file names.
    pub fn key(self) -> &'static str {
        match self {
            Example::Rectangle => "rectangle",
            Example::PlateWithHole => "plate-with-hole",
            Example::Channel => "channel",
            Example::LShape => "l-shape",
            Example::SurfaceErt => "surface-ert",
            Example::Crosshole => "crosshole",
            Example::Refraction => "refraction",
            Example::GravityMagnetics => "gravity-magnetics",
            Example::Magnetotelluric => "magnetotelluric",
            Example::AquiferWell => "aquifer-well",
            Example::Watershed => "watershed",
        }
    }

    pub fn from_key(key: &str) -> Option<Example> {
        Example::ALL.into_iter().find(|e| e.key() == key)
    }

    pub fn field(self) -> Field {
        match self {
            Example::Rectangle | Example::PlateWithHole | Example::Channel | Example::LShape => {
                Field::General
            }
            Example::AquiferWell | Example::Watershed => Field::Hydrology,
            _ => Field::Geophysics,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Example::Rectangle => "Rectangle",
            Example::PlateWithHole => "Plate with a hole and inclusions",
            Example::Channel => "Channel past a cylinder",
            Example::LShape => "L-shaped domain",
            Example::SurfaceErt => "Surface ERT line",
            Example::Crosshole => "Crosshole tomography",
            Example::Refraction => "Seismic refraction line",
            Example::GravityMagnetics => "Gravity or magnetics body",
            Example::Magnetotelluric => "Magnetotelluric profile",
            Example::AquiferWell => "Aquifer with a pumping well",
            Example::Watershed => "Watershed",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Example::Rectangle => "An empty rectangle with its four edges marked 1 to 4: a start for any geometry.",
            Example::PlateWithHole => {
                "A plate with a round hole and two inclusions of other materials, as for heat or stress problems."
            }
            Example::Channel => {
                "A channel with a cylinder in it, inlet, outlet, walls and cylinder marked apart, \
                 cells refined round the cylinder, as for flow problems."
            }
            Example::LShape => "An L-shaped domain with cells refined at its inner corner, where solutions are singular.",
            Example::SurfaceErt => {
                "Electrodes on a low hill, an inversion region under them and a wide outer region, laid out \
                 as pyGIMLi does."
            }
            Example::Crosshole => "Sensors in two boreholes (ERT, seismic or radar) and an inversion region between them.",
            Example::Refraction => {
                "Geophones on sloping ground; the mesh covers only the ground under the line, as traveltime \
                 tomography needs."
            }
            Example::GravityMagnetics => {
                "Stations over overburden, bedrock and a dipping ore body, in a domain wide enough for \
                 potential fields."
            }
            Example::Magnetotelluric => {
                "Stations on a ridge, air above, a conductor and a basement, with cells growing to a boundary \
                 far away."
            }
            Example::Watershed => {
                "An illustrative watershed in plan view, with a marked outlet, a branching stream network, \
                 and a riparian zone. Cells are refined along the streams."
            }
            Example::AquiferWell => {
                "Soil, aquifer and clay layers, a well with its screen marked for the pumping boundary, and an \
                 ERT line to monitor it."
            }
        }
    }

    pub fn params(self) -> Vec<Param> {
        match self {
            Example::Rectangle => vec![
                num("width", "Width", 100.0, 1e-6, 1e9),
                num("height", "Height", 50.0, 1e-6, 1e9),
            ],
            Example::PlateWithHole => vec![
                num("width", "Width", 10.0, 1e-6, 1e9),
                num("height", "Height", 5.0, 1e-6, 1e9),
                num("radius", "Hole radius", 1.0, 1e-6, 1e9),
            ],
            Example::Channel => vec![
                num("length", "Length", 22.0, 1e-6, 1e9),
                num("height", "Height", 4.1, 1e-6, 1e9),
                num("radius", "Cylinder radius", 0.5, 1e-6, 1e9),
            ],
            Example::LShape => vec![
                num("size", "Size", 2.0, 1e-6, 1e9),
                num("corner", "Cell size at the corner", 0.01, 1e-9, 1e9),
            ],
            Example::SurfaceErt => vec![
                count("electrodes", "Electrodes", 41.0, 2.0, 10_000.0),
                num("spacing", "Electrode spacing", 1.0, 1e-6, 1e6),
                num("hill", "Hill height", 1.5, -1e6, 1e6),
            ],
            Example::Crosshole => vec![
                count("sensors", "Sensors per borehole", 20.0, 1.0, 10_000.0),
                num("spacing", "Sensor spacing", 1.0, 1e-6, 1e6),
                num("distance", "Borehole distance", 10.0, 1e-6, 1e6),
            ],
            Example::Refraction => vec![
                count("geophones", "Geophones", 48.0, 2.0, 10_000.0),
                num("spacing", "Geophone spacing", 2.0, 1e-6, 1e6),
            ],
            Example::GravityMagnetics => vec![
                count("stations", "Stations", 41.0, 1.0, 10_000.0),
                num("spacing", "Station spacing", 10.0, 1e-6, 1e6),
                num("overburden", "Overburden thickness", 20.0, 1e-6, 1e6),
            ],
            Example::Magnetotelluric => vec![
                count("stations", "Stations", 21.0, 1.0, 10_000.0),
                num("spacing", "Station spacing", 500.0, 1e-6, 1e7),
                num("air", "Air height", 100_000.0, 1e-6, 1e9),
            ],
            Example::Watershed => vec![
                num("width", "Basin width (m)", 1200.0, 10.0, 1e6),
                num("length", "Basin length (m)", 1800.0, 10.0, 1e6),
                num("river_size", "River cell size (m)", 20.0, 0.1, 1e4),
            ],
            Example::AquiferWell => vec![
                num("aquifer", "Aquifer top depth", 4.0, 1e-6, 1e5),
                num("clay", "Clay top depth", 16.0, 1e-6, 1e5),
                num("screen_top", "Screen top depth", 8.0, 1e-6, 1e5),
                num("screen_bottom", "Screen bottom depth", 15.0, 1e-6, 1e5),
            ],
        }
    }

    pub fn defaults(self) -> Vec<f64> {
        self.params().iter().map(|p| p.default).collect()
    }

    /// The template with its default parameters.
    pub fn geometry(self) -> Geometry2D {
        self.build(&self.defaults())
    }

    /// The template with `values` (in the order of [`Self::params`]; missing
    /// ones take their default, all are clamped to their range).
    pub fn build(self, values: &[f64]) -> Geometry2D {
        let params = self.params();
        let v: Vec<f64> = params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let x = values
                    .get(i)
                    .copied()
                    .filter(|x| x.is_finite())
                    .unwrap_or(p.default)
                    .clamp(p.min, p.max);
                if p.whole {
                    x.round()
                } else {
                    x
                }
            })
            .collect();
        let mut g = match self {
            Example::Rectangle => rectangle_domain(v[0], v[1]),
            Example::PlateWithHole => plate(v[0], v[1], v[2]),
            Example::Channel => channel(v[0], v[1], v[2]),
            Example::LShape => l_shape(v[0], v[1]),
            Example::SurfaceErt => surface_ert(v[0] as usize, v[1], v[2]),
            Example::Crosshole => crosshole(v[0] as usize, v[1], v[2]),
            Example::Refraction => refraction(v[0] as usize, v[1]),
            Example::GravityMagnetics => gravity_magnetics(v[0] as usize, v[1], v[2]),
            Example::Magnetotelluric => magnetotelluric(v[0] as usize, v[1], v[2]),
            Example::AquiferWell => aquifer_well(v[0], v[1], v[2], v[3]),
            Example::Watershed => watershed(v[0], v[1], v[2]),
        };
        g.name = self.title().into();
        g
    }
}

// ---- Building blocks for cross-sections through the ground ----------------

/// Height of a profile (any order) at `x`, held flat beyond its ends.
pub fn height(profile: &[[f64; 2]], x: f64) -> f64 {
    let mut p = profile.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let (Some(first), Some(last)) = (p.first(), p.last()) else {
        return 0.0;
    };
    if x <= first[0] {
        return first[1];
    }
    if x >= last[0] {
        return last[1];
    }
    let i = p.partition_point(|q| q[0] <= x);
    let (a, b) = (p[i - 1], p[i]);
    a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0])
}

/// A profile from `x0` to `x1`, left to right.
pub fn across(profile: &[[f64; 2]], x0: f64, x1: f64) -> Vec<[f64; 2]> {
    let mut p = profile.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut out = vec![[x0, height(&p, x0)]];
    out.extend(p.iter().filter(|q| q[0] > x0 && q[0] < x1));
    out.push([x1, height(&p, x1)]);
    out
}

/// The ground from `x0` to `x1` down to `bottom` as a domain: the surface
/// edges carry `surface`, the sides and bottom `outer`.
pub fn ground_domain(
    surface: &[[f64; 2]],
    x0: f64,
    x1: f64,
    bottom: f64,
    surface_marker: i32,
    outer: i32,
) -> Domain {
    let mut top = across(surface, x0, x1);
    top.reverse();
    let mut outline = vec![[x0, bottom], [x1, bottom]];
    let mut edge_markers = vec![outer, outer];
    edge_markers.extend(std::iter::repeat_n(surface_marker, top.len() - 1));
    edge_markers.push(outer);
    outline.extend(top);
    Domain {
        outline,
        edge_markers,
    }
}

/// The ground from `x0` to `x1` down to `bottom` as a shape outline.
pub fn ground_polygon(surface: &[[f64; 2]], x0: f64, x1: f64, bottom: f64) -> Vec<[f64; 2]> {
    ground_domain(surface, x0, x1, bottom, 0, 0).outline
}

/// What lies between two profiles from `x0` to `x1`.
pub fn band(top: &[[f64; 2]], bottom: &[[f64; 2]], x0: f64, x1: f64) -> Vec<[f64; 2]> {
    let mut out = across(top, x0, x1);
    let mut lower = across(bottom, x0, x1);
    lower.reverse();
    out.extend(lower);
    out
}

fn profile(x0: f64, x1: f64, dx: f64, y: impl Fn(f64) -> f64) -> Vec<[f64; 2]> {
    let n = ((x1 - x0) / dx).round().max(1.0) as usize;
    (0..=n)
        .map(|i| x0 + (x1 - x0) * i as f64 / n as f64)
        .map(|x| [x, y(x)])
        .collect()
}

/// `n` sensors from `x0` every `dx` on the ground surface.
pub fn on_surface(surface: &[[f64; 2]], x0: f64, dx: f64, n: usize) -> Vec<Point> {
    (0..n)
        .map(|i| x0 + dx * i as f64)
        .map(|x| Point {
            at: [x, height(surface, x)],
            marker: SENSOR_MARKER,
            refine: true,
        })
        .collect()
}

// ---- General ---------------------------------------------------------------

fn rectangle_domain(w: f64, h: f64) -> Geometry2D {
    Geometry2D {
        domain: Domain::rectangle(0.0, 0.0, w, h),
        sizing: Sizing {
            max_area: w * h / 1000.0,
            ..Sizing::default()
        },
        ..Geometry2D::default()
    }
}

fn plate(w: f64, h: f64, r: f64) -> Geometry2D {
    let r = r.min(0.45 * h).min(0.2 * w);
    let mut g = Geometry2D {
        domain: Domain::rectangle(0.0, 0.0, w, h),
        background: Zone::new("Plate", 1, 0.0),
        sizing: Sizing {
            min_angle: 30.0,
            max_area: w * h / 2000.0,
            ..Sizing::default()
        },
        ..Geometry2D::default()
    };
    g.shapes.push(Shape {
        zone: Zone::new("Inclusion A", 2, 0.0),
        outline: ellipse([0.2 * w, 0.5 * h], 0.6 * r, 0.6 * r, 24),
        ..Shape::default()
    });
    g.shapes.push(Shape {
        zone: Zone::new("Inclusion B", 3, 0.0),
        outline: rectangle(0.7 * w, 0.3 * h, 0.85 * w, 0.7 * h),
        ..Shape::default()
    });
    g.shapes.push(Shape {
        zone: Zone::new("Hole", 0, 0.0),
        outline: ellipse([0.5 * w, 0.5 * h], r, r, 48),
        edge_marker: 5,
        hole: true,
    });
    g
}

fn channel(length: f64, h: f64, r: f64) -> Geometry2D {
    // Half a channel height behind the inlet, as in the usual benchmark.
    let r = r.min(0.4 * h).min(0.1 * length);
    let centre = [(0.5 * h).max(2.0 * r).min(0.25 * length), 0.5 * h];
    let cylinder = ellipse(centre, r, r, 64);
    let mut g = Geometry2D {
        // Bottom and top are walls (3), the left edge the inlet (1), the right the outlet (2).
        domain: Domain {
            outline: rectangle(0.0, 0.0, length, h),
            edge_markers: vec![3, 2, 3, 1],
        },
        background: Zone::new("Fluid", 1, 0.0),
        sizing: Sizing {
            min_angle: 30.0,
            max_area: length * h / 1500.0,
            point_size: r / 8.0,
            size_growth: 0.15,
        },
        ..Geometry2D::default()
    };
    g.points = cylinder
        .iter()
        .map(|&at| Point {
            at,
            marker: 0,
            refine: true,
        })
        .collect();
    g.shapes.push(Shape {
        zone: Zone::new("Cylinder", 0, 0.0),
        outline: cylinder,
        edge_marker: 4,
        hole: true,
    });
    g
}

fn l_shape(size: f64, corner: f64) -> Geometry2D {
    let s = size;
    Geometry2D {
        domain: Domain {
            outline: vec![
                [0.0, 0.0],
                [s, 0.0],
                [s, 0.5 * s],
                [0.5 * s, 0.5 * s],
                [0.5 * s, s],
                [0.0, s],
            ],
            edge_markers: vec![1; 6],
        },
        points: vec![Point {
            at: [0.5 * s, 0.5 * s],
            marker: 1,
            refine: true,
        }],
        sizing: Sizing {
            min_angle: 30.0,
            max_area: s * s / 400.0,
            point_size: corner.min(0.25 * s),
            size_growth: 0.3,
        },
        ..Geometry2D::default()
    }
}

// ---- Geophysics ------------------------------------------------------------

/// pyGIMLi's ERT layout (`createParaMeshPLC` defaults): the inversion region
/// reaches two spacings past the outer electrodes and 0.4 times the spread
/// down; the outer region four spreads further.
fn surface_ert(n: usize, a: f64, hill: f64) -> Geometry2D {
    let span = a * (n - 1) as f64;
    let hill = |x: f64| {
        if (0.0..=span).contains(&x) {
            0.5 * hill * (1.0 - (2.0 * PI * x / span).cos())
        } else {
            0.0
        }
    };
    let surface = profile(0.0, span, a, hill);
    let (para, depth, outer) = (2.0 * a, 0.4 * span, 4.0 * span);
    let (left, right) = (-para - outer, span + para + outer);
    let mut g = Geometry2D {
        domain: ground_domain(
            &surface,
            left,
            right,
            -depth - outer,
            SURFACE_MARKER,
            OUTER_MARKER,
        ),
        background: Zone::new("Outer region", 1, 0.0),
        sizing: Sizing {
            min_angle: 28.0,
            max_area: 0.0,
            point_size: 0.25 * a,
            size_growth: 0.3,
        },
        ..Geometry2D::default()
    };
    g.shapes.push(Shape {
        zone: Zone::new("Inversion region", 2, 0.0),
        outline: ground_polygon(&surface, -para, span + para, -depth),
        ..Shape::default()
    });
    g.points = on_surface(&surface, 0.0, a, n);
    g
}

fn crosshole(n: usize, dz: f64, distance: f64) -> Geometry2D {
    let depth = dz * (n + 1) as f64;
    let pad = 4.0 * distance.max(depth);
    let flat = [[0.0, 0.0]];
    let mut g = Geometry2D {
        domain: ground_domain(
            &flat,
            -pad,
            distance + pad,
            -depth - pad,
            SURFACE_MARKER,
            OUTER_MARKER,
        ),
        background: Zone::new("Outer region", 1, 0.0),
        sizing: Sizing {
            min_angle: 28.0,
            max_area: 0.0,
            point_size: 0.25 * dz,
            size_growth: 0.3,
        },
        ..Geometry2D::default()
    };
    g.shapes.push(Shape {
        zone: Zone::new("Inversion region", 2, 0.0),
        outline: rectangle(-2.0 * dz, -depth - 2.0 * dz, distance + 2.0 * dz, 0.0),
        ..Shape::default()
    });
    for (i, x) in [0.0, distance].into_iter().enumerate() {
        g.lines.push(Line {
            name: format!("Borehole {}", i + 1),
            points: vec![[x, 0.0], [x, -depth]],
            marker: 0,
        });
        g.points.extend(points_along(
            [x, -dz],
            [x, -dz * n as f64],
            n,
            SENSOR_MARKER,
        ));
    }
    g
}

/// pyGIMLi's traveltime layout: no outer region, the mesh ends under the
/// outer geophones.
fn refraction(n: usize, a: f64) -> Geometry2D {
    let span = a * (n - 1) as f64;
    let surface = profile(0.0, span, a, |x| 0.06 * x + 1.5 * (PI * x / span).sin());
    let lowest = surface.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    Geometry2D {
        domain: ground_domain(
            &surface,
            0.0,
            span,
            lowest - 0.4 * span,
            SURFACE_MARKER,
            OUTER_MARKER,
        ),
        background: Zone::new("Velocity model", 2, 0.0),
        points: on_surface(&surface, 0.0, a, n),
        sizing: Sizing {
            min_angle: 28.0,
            max_area: 0.0,
            point_size: 0.5 * a,
            size_growth: 0.2,
        },
        ..Geometry2D::default()
    }
}

fn gravity_magnetics(n: usize, dx: f64, overburden: f64) -> Geometry2D {
    let half = 0.5 * dx * (n.max(2) - 1) as f64;
    let size = 5.0 * half.max(10.0 * overburden);
    let flat = [[0.0, 0.0]];
    let d = overburden;
    let mut g = Geometry2D {
        domain: ground_domain(&flat, -size, size, -size, SURFACE_MARKER, OUTER_MARKER),
        background: Zone::new("Overburden", 1, 0.0),
        sizing: Sizing {
            min_angle: 28.0,
            max_area: 0.0,
            point_size: 0.5 * dx,
            size_growth: 0.3,
        },
        ..Geometry2D::default()
    };
    g.shapes.push(Shape {
        zone: Zone::new("Bedrock", 2, 0.0),
        outline: band(&[[0.0, -d]], &[[0.0, -size]], -size, size),
        ..Shape::default()
    });
    g.shapes.push(Shape {
        zone: Zone::new("Ore body", 3, d * d / 20.0),
        outline: vec![
            [-1.5 * d, -2.0 * d],
            [0.5 * d, -2.0 * d],
            [2.0 * d, -4.5 * d],
            [0.0, -4.5 * d],
        ],
        ..Shape::default()
    });
    g.points = on_surface(&flat, -half, dx, n);
    g
}

fn magnetotelluric(n: usize, dx: f64, air: f64) -> Geometry2D {
    let half = 0.5 * dx * (n.max(2) - 1) as f64;
    let size = 20.0 * half;
    let ridge = |x: f64| 0.03 * half * (-(x / (0.6 * half)).powi(2)).exp();
    let surface = profile(-2.0 * half, 2.0 * half, 0.5 * dx, ridge);
    let top = surface.iter().map(|p| p[1]).fold(0.0, f64::max) + air;
    let mut g = Geometry2D {
        domain: Domain {
            outline: rectangle(-size, -size, size, top),
            edge_markers: vec![OUTER_MARKER; 4],
        },
        background: Zone::new("Air", 0, 0.0),
        sizing: Sizing {
            min_angle: 28.0,
            max_area: 0.0,
            point_size: 0.2 * dx,
            size_growth: 0.35,
        },
        ..Geometry2D::default()
    };
    g.shapes.push(Shape {
        zone: Zone::new("Host rock", 1, 0.0),
        outline: ground_polygon(&surface, -size, size, -size),
        edge_marker: SURFACE_MARKER,
        hole: false,
    });
    g.shapes.push(Shape {
        zone: Zone::new("Basement", 3, 0.0),
        outline: band(&[[0.0, -1.6 * half]], &[[0.0, -size]], -size, size),
        ..Shape::default()
    });
    g.shapes.push(Shape {
        zone: Zone::new("Conductor", 2, 0.0),
        outline: ellipse([0.3 * half, -0.5 * half], 0.3 * half, 0.12 * half, 32),
        ..Shape::default()
    });
    g.points = on_surface(&surface, -half, dx, n);
    g
}

// ---- Hydrology -----------------------------------------------------------

fn aquifer_well(aquifer: f64, clay: f64, screen_top: f64, screen_bottom: f64) -> Geometry2D {
    let clay = clay.max(aquifer + 0.5);
    let lower = clay + 4.0;
    let bottom = -(lower + 10.0);
    let (screen_top, screen_bottom) = (
        screen_top.max(aquifer),
        screen_bottom.min(clay).max(screen_top + 0.5),
    );
    let surface = [[-100.0, 1.0], [100.0, -1.0]];
    let mut g = Geometry2D {
        domain: ground_domain(
            &surface,
            -100.0,
            100.0,
            bottom,
            SURFACE_MARKER,
            OUTER_MARKER,
        ),
        background: Zone::new("Unsaturated soil", 1, 0.0),
        sizing: Sizing {
            min_angle: 28.0,
            max_area: 0.0,
            point_size: 0.4,
            size_growth: 0.25,
        },
        ..Geometry2D::default()
    };
    let deep = [[0.0, bottom]];
    for (name, marker, top) in [
        ("Sand aquifer", 2, aquifer),
        ("Clay aquitard", 3, clay),
        ("Lower aquifer", 4, lower),
    ] {
        g.shapes.push(Shape {
            zone: Zone::new(name, marker, 0.0),
            outline: band(&[[0.0, -top]], &deep, -100.0, 100.0),
            ..Shape::default()
        });
    }
    let head = [0.0, height(&surface, 0.0)];
    g.lines.push(Line {
        name: "Well casing".into(),
        points: vec![head, [0.0, -screen_top]],
        marker: 0,
    });
    g.lines.push(Line {
        name: "Well screen".into(),
        points: vec![[0.0, -screen_top], [0.0, -screen_bottom]],
        marker: 10,
    });
    g.points = on_surface(&surface, -15.5, 1.0, 32);
    let n = ((screen_bottom - screen_top).round() as usize).max(1) + 1;
    g.points.extend(points_along(
        [0.0, -screen_top],
        [0.0, -screen_bottom],
        n,
        0,
    ));
    g
}

/// An illustrative plan-view catchment, not a watershed delineated from a DEM.
/// Edge markers: divide 1, outlet 2, main stream 10, tributaries 11,
/// riparian-zone boundary 12. The outlet node is marked 20.
fn watershed(width: f64, length: f64, river_size: f64) -> Geometry2D {
    let xy = |p: [f64; 2]| [p[0] * width, p[1] * length];
    let scale = width.min(length);
    // Keep even extreme template parameters within a useful interactive budget.
    let river_size = river_size.clamp(0.002 * scale, 0.1 * scale);
    let outline = [
        [-0.04, 0.0],
        [0.04, 0.0],
        [0.28, 0.18],
        [0.5, 0.5],
        [0.42, 0.8],
        [0.16, 1.0],
        [-0.2, 0.96],
        [-0.48, 0.7],
        [-0.44, 0.35],
        [-0.22, 0.14],
    ]
    .map(xy)
    .to_vec();
    let mut markers = vec![1; outline.len()];
    markers[0] = 2;
    let main = [
        [0.0, 0.0],
        [-0.025, 0.2],
        [0.035, 0.43],
        [0.0, 0.65],
        [0.07, 0.88],
    ];
    let mut g = Geometry2D {
        domain: Domain {
            outline,
            edge_markers: markers,
        },
        background: Zone::new("Uplands", 1, 0.0),
        sizing: Sizing {
            min_angle: 28.0,
            max_area: scale * scale / 600.0,
            point_size: river_size,
            size_growth: 0.3,
        },
        ..Geometry2D::default()
    };
    let corridor: Vec<_> = main
        .iter()
        .map(|p| xy([p[0] - 0.025, p[1]]))
        .chain(main.iter().rev().map(|p| xy([p[0] + 0.025, p[1]])))
        .collect();
    g.shapes.push(Shape {
        zone: Zone::new("Riparian zone", 2, river_size * river_size * 2.0),
        outline: corridor,
        edge_marker: 12,
        hole: false,
    });
    g.lines.push(Line {
        name: "Main stream".into(),
        points: main.map(xy).to_vec(),
        marker: 10,
    });
    for (name, points) in [
        ("West headwater", vec![[-0.3, 0.78], [-0.16, 0.72], main[3]]),
        ("East tributary", vec![[0.33, 0.67], [0.2, 0.55], main[2]]),
        ("West tributary", vec![[-0.28, 0.4], [-0.17, 0.3], main[1]]),
    ] {
        g.lines.push(Line {
            name: name.into(),
            points: points.into_iter().map(xy).collect(),
            marker: 11,
        });
    }
    for line in &g.lines {
        for segment in line.points.windows(2) {
            let distance = (segment[1][0] - segment[0][0]).hypot(segment[1][1] - segment[0][1]);
            let n = ((distance / (2.0 * river_size)).ceil() as usize + 1).clamp(2, 256);
            // Avoid duplicate confluence/end points: those are already constrained by the lines.
            g.points.extend(
                points_along(segment[0], segment[1], n, 0)
                    .into_iter()
                    .skip(1)
                    .take(n - 2),
            );
        }
    }
    g.points.push(Point {
        at: xy(main[0]),
        marker: 20,
        refine: true,
    });
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_meshes_with_all_its_zones_holes_and_points() {
        for example in Example::ALL {
            let g = example.geometry();
            let mesh = g
                .mesh(2_000_000)
                .unwrap_or_else(|e| panic!("{}: {e}", example.title()));
            assert!(
                mesh.report.refinement_complete,
                "{}: {:?}",
                example.title(),
                mesh.report
            );
            assert_eq!(mesh.report.area_violations, 0);
            assert_eq!(mesh.report.complete, mesh.report.angle_violations == 0);
            assert!(
                mesh.report.warnings.is_empty(),
                "{}: {:?}",
                example.title(),
                mesh.report.warnings
            );
            for zone in g.zones() {
                assert!(
                    mesh.cell_markers.contains(&zone.marker),
                    "{}: no cells in {}",
                    example.title(),
                    zone.name
                );
            }
            for p in &g.points {
                let found = mesh.nodes.iter().zip(&mesh.node_markers).any(|(q, &m)| {
                    (q[0] - p.at[0]).abs() < 1e-6 && (q[1] - p.at[1]).abs() < 1e-6 && m == p.marker
                });
                assert!(
                    found,
                    "{}: point at {:?} is not a node",
                    example.title(),
                    p.at
                );
            }
            for hole in g.shapes.iter().filter(|s| s.hole) {
                let centre = hole
                    .outline
                    .iter()
                    .fold([0.0, 0.0], |c, p| [c[0] + p[0], c[1] + p[1]]);
                let centre = [
                    centre[0] / hole.outline.len() as f64,
                    centre[1] / hole.outline.len() as f64,
                ];
                assert!(
                    g.zone_at(centre).is_none(),
                    "{}: hole centre has a zone",
                    example.title()
                );
                let covered = mesh.triangles.iter().any(|t| {
                    let [a, b, c] = t.map(|k| mesh.nodes[k]);
                    let s = |p: [f64; 2], q: [f64; 2]| {
                        (q[0] - p[0]) * (centre[1] - p[1]) - (q[1] - p[1]) * (centre[0] - p[0])
                    };
                    s(a, b) > 0.0 && s(b, c) > 0.0 && s(c, a) > 0.0
                });
                assert!(!covered, "{}: a cell covers the hole", example.title());
            }
        }
    }
}
