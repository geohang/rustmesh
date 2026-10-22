//! Point lists as editable text: one point per line, numbers separated by
//! commas, semicolons, tabs or spaces, so a spreadsheet column pastes
//! straight in. Lines starting with `#` and a first line of column names are
//! skipped.

use rustmesh::Point;

/// The numbers on each data line, with its 1-based line number.
fn rows(text: &str) -> Result<Vec<(usize, Vec<f64>)>, String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line
            .split([',', ';', '\t', ' '])
            .filter(|f| !f.is_empty())
            .collect();
        match fields
            .iter()
            .map(|f| f.parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(values) if values.iter().all(|v| v.is_finite()) => out.push((i + 1, values)),
            _ if out.is_empty() && fields.iter().any(|f| f.chars().any(char::is_alphabetic)) => {}
            _ => {
                return Err(format!(
                    "line {}: \"{line}\" is not a list of numbers",
                    i + 1
                ))
            }
        }
    }
    Ok(out)
}

fn marker(value: f64, line: usize) -> Result<i32, String> {
    if value.fract() == 0.0 && value.abs() < i32::MAX as f64 {
        Ok(value as i32)
    } else {
        Err(format!("line {line}: a marker must be a whole number"))
    }
}

/// `x, y` per line.
pub fn xy_to_text(points: &[[f64; 2]]) -> String {
    points
        .iter()
        .map(|p| format!("{}, {}\n", p[0], p[1]))
        .collect()
}

pub fn parse_xy(text: &str) -> Result<Vec<[f64; 2]>, String> {
    rows(text)?
        .into_iter()
        .map(|(line, v)| match v[..] {
            [x, y] => Ok([x, y]),
            _ => Err(format!("line {line}: expected two numbers, x and y")),
        })
        .collect()
}

/// `x, y, marker` per corner, the marker belonging to the edge to the next corner.
pub fn outline_to_text(points: &[[f64; 2]], markers: &[i32]) -> String {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            format!(
                "{}, {}, {}\n",
                p[0],
                p[1],
                markers.get(i).copied().unwrap_or(0)
            )
        })
        .collect()
}

pub fn parse_outline(text: &str) -> Result<(Vec<[f64; 2]>, Vec<i32>), String> {
    let mut points = Vec::new();
    let mut markers = Vec::new();
    for (line, v) in rows(text)? {
        match v[..] {
            [x, y] => {
                points.push([x, y]);
                markers.push(0);
            }
            [x, y, m] => {
                points.push([x, y]);
                markers.push(marker(m, line)?);
            }
            _ => {
                return Err(format!(
                    "line {line}: expected x, y and optionally an edge marker"
                ))
            }
        }
    }
    Ok((points, markers))
}

/// `x, y, marker` per point, and a fourth column 0 for a point without refinement.
pub fn points_to_text(points: &[Point]) -> String {
    points
        .iter()
        .map(|p| {
            if p.refine {
                format!("{}, {}, {}\n", p.at[0], p.at[1], p.marker)
            } else {
                format!("{}, {}, {}, 0\n", p.at[0], p.at[1], p.marker)
            }
        })
        .collect()
}

pub fn parse_points(text: &str) -> Result<Vec<Point>, String> {
    rows(text)?
        .into_iter()
        .map(|(line, v)| match v[..] {
            [x, y] => Ok(Point {
                at: [x, y],
                marker: 0,
                refine: true,
            }),
            [x, y, m] => Ok(Point {
                at: [x, y],
                marker: marker(m, line)?,
                refine: true,
            }),
            [x, y, m, r] => Ok(Point {
                at: [x, y],
                marker: marker(m, line)?,
                refine: r != 0.0,
            }),
            _ => Err(format!(
                "line {line}: expected x, y and optionally a marker and 1/0 for refinement"
            )),
        })
        .collect()
}
