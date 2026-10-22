//! 2D mesh files other programs read. `(x, z)` is written as the x-y plane.

use std::io::{self, Write};

use crate::mesh2d::Mesh2D;

/// VTK XML unstructured grid for ParaView: per cell the marker, region,
/// area and smallest angle; per node the marker; the marked edges as lines.
pub fn write_vtu<W: Write + ?Sized>(mesh: &Mesh2D, w: &mut W) -> io::Result<()> {
    let marked: Vec<usize> = (0..mesh.edges.len())
        .filter(|&e| mesh.edge_markers[e] != 0)
        .collect();
    let cells = mesh.triangles.len() + marked.len();
    writeln!(w, r#"<?xml version="1.0"?>"#)?;
    writeln!(
        w,
        r#"<VTKFile type="UnstructuredGrid" version="0.1" byte_order="LittleEndian">"#
    )?;
    writeln!(w, "<UnstructuredGrid>")?;
    writeln!(
        w,
        r#"<Piece NumberOfPoints="{}" NumberOfCells="{cells}">"#,
        mesh.nodes.len()
    )?;
    writeln!(
        w,
        r#"<Points><DataArray type="Float64" NumberOfComponents="3" format="ascii">"#
    )?;
    for p in &mesh.nodes {
        writeln!(w, "{} {} 0", p[0], p[1])?;
    }
    writeln!(w, "</DataArray></Points>")?;
    writeln!(w, "<Cells>")?;
    writeln!(
        w,
        r#"<DataArray type="Int64" Name="connectivity" format="ascii">"#
    )?;
    for t in &mesh.triangles {
        writeln!(w, "{} {} {}", t[0], t[1], t[2])?;
    }
    for &e in &marked {
        writeln!(w, "{} {}", mesh.edges[e][0], mesh.edges[e][1])?;
    }
    writeln!(w, "</DataArray>")?;
    writeln!(
        w,
        r#"<DataArray type="Int64" Name="offsets" format="ascii">"#
    )?;
    let lines = (1..=marked.len()).map(|k| 3 * mesh.triangles.len() + 2 * k);
    for offset in (1..=mesh.triangles.len()).map(|k| 3 * k).chain(lines) {
        writeln!(w, "{offset}")?;
    }
    writeln!(w, "</DataArray>")?;
    writeln!(w, r#"<DataArray type="UInt8" Name="types" format="ascii">"#)?;
    let types =
        std::iter::repeat_n(5, mesh.triangles.len()).chain(std::iter::repeat_n(3, marked.len()));
    for t in types {
        writeln!(w, "{t}")?;
    }
    writeln!(w, "</DataArray>")?;
    writeln!(w, "</Cells>")?;
    writeln!(w, "<CellData>")?;
    let edge_markers = marked.iter().map(|&e| mesh.edge_markers[e]);
    write_cell_data(
        w,
        "marker",
        "Int32",
        mesh.cell_markers.iter().copied().chain(edge_markers),
    )?;
    let none = || std::iter::repeat_n(-1, marked.len());
    write_cell_data(
        w,
        "region",
        "Int32",
        mesh.cell_regions.iter().copied().chain(none()),
    )?;
    // -1 on the edges: not every VTK reader takes NaN in ASCII.
    let unset = || std::iter::repeat_n(-1.0, marked.len());
    write_cell_data(
        w,
        "area",
        "Float64",
        mesh.cell_areas().into_iter().chain(unset()),
    )?;
    write_cell_data(
        w,
        "min_angle",
        "Float64",
        mesh.cell_min_angles().into_iter().chain(unset()),
    )?;
    writeln!(w, "</CellData>")?;
    writeln!(w, "<PointData>")?;
    write_cell_data(w, "marker", "Int32", mesh.node_markers.iter().copied())?;
    writeln!(w, "</PointData>")?;
    writeln!(w, "</Piece>\n</UnstructuredGrid>\n</VTKFile>")
}

fn write_cell_data<W: Write + ?Sized, T: std::fmt::Display>(
    w: &mut W,
    name: &str,
    kind: &str,
    values: impl Iterator<Item = T>,
) -> io::Result<()> {
    writeln!(
        w,
        r#"<DataArray type="{kind}" Name="{name}" format="ascii">"#
    )?;
    for v in values {
        writeln!(w, "{v}")?;
    }
    writeln!(w, "</DataArray>")
}

/// Gmsh MSH 2.2 ASCII, written for pyGIMLi's `meshtools.readGmsh`: Gmsh
/// physical numbers must be positive, so negative node markers (sensors,
/// -99) are written as points with the number negated, and edge markers -1
/// to -4 as lines 1 to 4, which pyGIMLi reads back as -1 to -4. Other edges
/// with a marker keep its absolute value; unmarked edges are left out.
/// Triangles carry the cell marker as their physical number. All nonzero
/// node markers are emitted. NodeData/ElementData also store original signed
/// markers by file entity ID; physical tags alone are not a lossless mapping.
pub fn write_gmsh<W: Write + ?Sized>(mesh: &Mesh2D, w: &mut W) -> io::Result<()> {
    writeln!(w, "$MeshFormat\n2.2 0 8\n$EndMeshFormat")?;
    writeln!(w, "$Nodes\n{}", mesh.nodes.len())?;
    for (i, p) in mesh.nodes.iter().enumerate() {
        writeln!(w, "{} {} {} 0", i + 1, p[0], p[1])?;
    }
    writeln!(w, "$EndNodes")?;
    let points: Vec<usize> = (0..mesh.nodes.len())
        .filter(|&n| mesh.node_markers[n] != 0)
        .collect();
    let lines: Vec<usize> = (0..mesh.edges.len())
        .filter(|&e| mesh.edge_markers[e] != 0)
        .collect();
    writeln!(
        w,
        "$Elements\n{}",
        points.len() + lines.len() + mesh.triangles.len()
    )?;
    let mut id = 0;
    for &n in &points {
        id += 1;
        let tag = mesh.node_markers[n].unsigned_abs();
        writeln!(w, "{id} 15 2 {tag} {tag} {}", n + 1)?;
    }
    for &e in &lines {
        id += 1;
        let tag = mesh.edge_markers[e].unsigned_abs();
        writeln!(
            w,
            "{id} 1 2 {tag} {tag} {} {}",
            mesh.edges[e][0] + 1,
            mesh.edges[e][1] + 1
        )?;
    }
    for (t, tri) in mesh.triangles.iter().enumerate() {
        id += 1;
        let (marker, region) = (
            mesh.cell_markers[t].unsigned_abs(),
            mesh.cell_regions[t] + 1,
        );
        writeln!(
            w,
            "{id} 2 2 {marker} {region} {} {} {}",
            tri[0] + 1,
            tri[1] + 1,
            tri[2] + 1
        )?;
    }
    writeln!(w, "$EndElements")?;
    write_gmsh_data(
        w,
        "NodeData",
        "rustmesh:node_marker",
        mesh.node_markers.iter().copied(),
    )?;
    write_gmsh_data(
        w,
        "ElementData",
        "rustmesh:element_marker",
        points
            .iter()
            .map(|&n| mesh.node_markers[n])
            .chain(lines.iter().map(|&e| mesh.edge_markers[e]))
            .chain(mesh.cell_markers.iter().copied()),
    )
}

/// Standard MSH scalar datasets; entry IDs match $Nodes/$Elements exactly.
fn write_gmsh_data<W: Write + ?Sized>(
    w: &mut W,
    section: &str,
    name: &str,
    values: impl Iterator<Item = i32>,
) -> io::Result<()> {
    let values: Vec<_> = values.collect();
    writeln!(
        w,
        "${section}\n1\n\"{name}\"\n1\n0\n3\n0\n1\n{}",
        values.len()
    )?;
    for (i, value) in values.iter().enumerate() {
        writeln!(w, "{} {value}", i + 1)?;
    }
    writeln!(w, "$End{section}")
}

/// 3D VTK XML: tetrahedra and recovered triangular facets, with original
/// signed cell and triangle markers, region indices and node markers kept as they are.
pub fn write_vtu3d<W: Write + ?Sized>(mesh: &crate::Mesh3D, w: &mut W) -> io::Result<()> {
    let nt = mesh.tetrahedra.len();
    let nf = mesh.faces.len();
    writeln!(w, r#"<?xml version="1.0"?>"#)?;
    writeln!(
        w,
        r#"<VTKFile type="UnstructuredGrid" version="0.1" byte_order="LittleEndian"><UnstructuredGrid>"#
    )?;
    writeln!(
        w,
        r#"<Piece NumberOfPoints="{}" NumberOfCells="{}">"#,
        mesh.nodes.len(),
        nt + nf
    )?;
    writeln!(
        w,
        r#"<Points><DataArray type="Float64" NumberOfComponents="3" format="ascii">"#
    )?;
    for p in &mesh.nodes {
        writeln!(w, "{} {} {}", p[0], p[1], p[2])?;
    }
    writeln!(w, "</DataArray></Points><Cells>")?;
    writeln!(
        w,
        r#"<DataArray type="Int64" Name="connectivity" format="ascii">"#
    )?;
    for t in &mesh.tetrahedra {
        writeln!(w, "{} {} {} {}", t[0], t[1], t[2], t[3])?;
    }
    for f in &mesh.faces {
        writeln!(w, "{} {} {}", f[0], f[1], f[2])?;
    }
    writeln!(w, "</DataArray>")?;
    write_cell_data(
        w,
        "offsets",
        "Int64",
        (1..=nt)
            .map(|k| 4 * k)
            .chain((1..=nf).map(|k| 4 * nt + 3 * k)),
    )?;
    write_cell_data(
        w,
        "types",
        "UInt8",
        std::iter::repeat_n(10, nt).chain(std::iter::repeat_n(5, nf)),
    )?;
    writeln!(w, "</Cells><CellData>")?;
    write_cell_data(
        w,
        "marker",
        "Int32",
        mesh.cell_markers
            .iter()
            .copied()
            .chain(mesh.face_markers.iter().copied()),
    )?;
    write_cell_data(
        w,
        "region",
        "Int32",
        mesh.cell_regions
            .iter()
            .copied()
            .chain(std::iter::repeat_n(-1, nf)),
    )?;
    writeln!(w, "</CellData><PointData>")?;
    write_cell_data(w, "marker", "Int32", mesh.node_markers.iter().copied())?;
    writeln!(w, "</PointData></Piece></UnstructuredGrid></VTKFile>")
}

/// Gmsh 2.2 ASCII tetrahedra and triangular constraints. Physical tags use
/// absolute marker values. The original signed markers are stored in scalar
/// NodeData/ElementData, keyed by the node/element IDs in this file.
pub fn write_gmsh3d<W: Write + ?Sized>(mesh: &crate::Mesh3D, w: &mut W) -> io::Result<()> {
    writeln!(
        w,
        "$MeshFormat\n2.2 0 8\n$EndMeshFormat\n$Nodes\n{}",
        mesh.nodes.len()
    )?;
    for (i, p) in mesh.nodes.iter().enumerate() {
        writeln!(w, "{} {} {} {}", i + 1, p[0], p[1], p[2])?;
    }
    let marked: Vec<_> = mesh
        .node_markers
        .iter()
        .enumerate()
        .filter(|(_, m)| **m != 0)
        .collect();
    writeln!(
        w,
        "$EndNodes\n$Elements\n{}",
        mesh.tetrahedra.len() + mesh.faces.len() + marked.len()
    )?;
    let mut id = 0;
    for &(i, marker) in &marked {
        id += 1;
        let tag = marker.unsigned_abs();
        writeln!(w, "{id} 15 2 {tag} {tag} {}", i + 1)?;
    }
    for (i, f) in mesh.faces.iter().enumerate() {
        id += 1;
        let tag = mesh.face_markers[i].unsigned_abs();
        writeln!(
            w,
            "{id} 2 2 {tag} {tag} {} {} {}",
            f[0] + 1,
            f[1] + 1,
            f[2] + 1
        )?;
    }
    for (i, t) in mesh.tetrahedra.iter().enumerate() {
        id += 1;
        let tag = mesh.cell_markers[i].unsigned_abs();
        let region = mesh.cell_regions[i] + 1;
        writeln!(
            w,
            "{id} 4 2 {tag} {region} {} {} {} {}",
            t[0] + 1,
            t[1] + 1,
            t[2] + 1,
            t[3] + 1
        )?;
    }
    writeln!(w, "$EndElements")?;
    write_gmsh_data(
        w,
        "NodeData",
        "rustmesh:node_marker",
        mesh.node_markers.iter().copied(),
    )?;
    write_gmsh_data(
        w,
        "ElementData",
        "rustmesh:element_marker",
        marked
            .iter()
            .map(|&(_, &m)| m)
            .chain(mesh.face_markers.iter().copied())
            .chain(mesh.cell_markers.iter().copied()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dataset(text: &str, section: &str) -> Vec<i32> {
        let body = text.split(&format!("${section}\n")).nth(1).unwrap();
        let lines: Vec<_> = body.lines().collect();
        let count: usize = lines[7].parse().unwrap();
        lines[8..8 + count]
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let (id, value) = line.split_once(' ').unwrap();
                assert_eq!(id.parse::<usize>().unwrap(), i + 1);
                value.parse().unwrap()
            })
            .collect()
    }

    #[test]
    fn signed_markers_do_not_collide_in_either_dimension() {
        let nodes = vec![20, -20, -99, i32::MIN];
        let mesh = Mesh2D {
            nodes: vec![[0.0, 0.0]; 4],
            node_markers: nodes.clone(),
            edges: vec![[0, 1], [1, 2], [2, 3]],
            edge_markers: vec![1, -1, i32::MIN],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            cell_markers: vec![7, -7],
            cell_regions: vec![0, 1],
            ..Mesh2D::default()
        };
        let mut bytes = Vec::new();
        write_gmsh(&mesh, &mut bytes).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(dataset(&text, "NodeData"), nodes);
        assert_eq!(
            dataset(&text, "ElementData"),
            [20, -20, -99, i32::MIN, 1, -1, i32::MIN, 7, -7]
        );
        let mesh = crate::Mesh3D {
            nodes: vec![[0.0; 3]; 4],
            node_markers: nodes.clone(),
            faces: vec![[0, 1, 2], [0, 2, 3]],
            face_markers: vec![3, -3],
            tetrahedra: vec![[0, 1, 2, 3]],
            cell_markers: vec![-7],
            cell_regions: vec![0],
            ..crate::Mesh3D::default()
        };
        let mut bytes = Vec::new();
        write_gmsh3d(&mesh, &mut bytes).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(dataset(&text, "NodeData"), nodes);
        assert_eq!(
            dataset(&text, "ElementData"),
            [20, -20, -99, i32::MIN, 3, -3, -7]
        );
    }
}
