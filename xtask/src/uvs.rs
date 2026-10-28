//! Measures how a model's texture coordinates line up with its geometry.
//!
//! A surface whose UVs are turned against its vertices has the texture painted sideways,
//! and that is invisible in summary statistics. For each geometry this reports the
//! covariance of each UV axis with each position axis. On upright geometry `v` tracks
//! height - windows stack upward - so a dominant `u`/height term means the UVs are ninety
//! degrees round.

use anyhow::{Context, Result};
use vc_stream::Streamer;
use vc_transport::{GameRoot, ImageArchive};
use vc_world::ObjectDefinition;

#[expect(
    clippy::many_single_char_names,
    reason = "u, v, x, y and z are the names these quantities have in the format"
)]
pub(crate) fn run(model: &str) -> Result<()> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let mut streamer = Streamer::open(archive);
    let _ = streamer.warm_textures();

    let definition = ObjectDefinition {
        id: 0,
        model: model.to_owned(),
        txd: "generic".to_owned(),
        draw_distance: 300.0,
        flags: 0,
    };
    let cooked = streamer
        .model(&definition)
        .with_context(|| format!("cooking {model}"))?;

    println!("{model}: {} geometries", cooked.meshes.len());

    for (index, mesh) in cooked.meshes.iter().enumerate() {
        let count = mesh.vertices.len();

        if count < 3 {
            continue;
        }

        let mut mean = [0.0f64; 5];
        for vertex in &mesh.vertices {
            mean[0] += f64::from(vertex.uv[0][0]);
            mean[1] += f64::from(vertex.uv[0][1]);
            mean[2] += f64::from(vertex.position[0]);
            mean[3] += f64::from(vertex.position[1]);
            mean[4] += f64::from(vertex.position[2]);
        }
        for slot in &mut mean {
            *slot /= count as f64;
        }

        let mut covariance = [0.0f64; 6];
        for vertex in &mesh.vertices {
            let u = f64::from(vertex.uv[0][0]) - mean[0];
            let v = f64::from(vertex.uv[0][1]) - mean[1];
            let x = f64::from(vertex.position[0]) - mean[2];
            let y = f64::from(vertex.position[1]) - mean[3];
            let z = f64::from(vertex.position[2]) - mean[4];

            covariance[0] = u.mul_add(x, covariance[0]);
            covariance[1] = u.mul_add(y, covariance[1]);
            covariance[2] = u.mul_add(z, covariance[2]);
            covariance[3] = v.mul_add(x, covariance[3]);
            covariance[4] = v.mul_add(y, covariance[4]);
            covariance[5] = v.mul_add(z, covariance[5]);
        }

        println!(
            "  geometry {index}: {count} vertices, {} groups",
            mesh.groups.len()
        );
        println!(
            "    cov(u,x) {:+8.1}  cov(u,y) {:+8.1}  cov(u,z) {:+8.1}",
            covariance[0], covariance[1], covariance[2]
        );
        println!(
            "    cov(v,x) {:+8.1}  cov(v,y) {:+8.1}  cov(v,z) {:+8.1}   {}",
            covariance[3],
            covariance[4],
            covariance[5],
            verdict(covariance[2], covariance[5]),
        );
    }

    Ok(())
}

fn verdict(u_height: f64, v_height: f64) -> &'static str {
    if u_height.abs() > v_height.abs() * 1.5 {
        "u tracks height - UVs rotated"
    } else if v_height.abs() > u_height.abs() * 1.5 {
        "v tracks height - as expected"
    } else {
        "height term weak on both axes"
    }
}
