//! TODO: example using ChunkMap2

use super::PosNormMesh;

use building_blocks_core::prelude::*;
use building_blocks_storage::{access::GetUncheckedRefRelease, prelude::*};

pub trait Height {
    fn height(&self) -> f32;
}

/// The output buffers used by `surface_nets`. These buffers can be cleared and reused without
/// reallocating memory.
#[derive(Default)]
pub struct HeightMapMeshBuffer {
    /// The isosurface positions and normals. Parallel to `surface_points`. The normals are *not*
    /// normalized, since that is done most efficiently on the GPU.
    pub mesh: PosNormMesh,

    // Used to map back from voxel stride to vertex index.
    stride_to_index: Vec<usize>,
}

impl HeightMapMeshBuffer {
    /// Clears all of the buffers, but keeps the memory allocated for reuse.
    pub fn reset(&mut self, array_size: usize) {
        self.mesh.clear();

        // Just make sure this buffer is long enough, whether or not we've used it before.
        self.stride_to_index.resize(array_size, 0);
    }
}

/// Generates a mesh with a vertex at each point on the interior of `extent`. Surface normals are
/// estimated using central differencing, which requires each vertex to have a complete Von Neumann
/// neighborhood. This means that points on the boundary of `extent` are not eligible as mesh
/// vertices, but they are still required.
///
/// This is illustrated in the ascii art below, where "b" is a boundary point and "i" is an interior
/// point. Line segments denote the edges of the mesh.
///
/// ```text
/// b   b   b   b
///
/// b   i - i   b
///     | / |
/// b   i - i   b
///
/// b   b   b   b
/// ```
pub fn triangulate_height_map<H>(
    height_map: &Array2<H>,
    extent: &Extent2i,
    output: &mut HeightMapMeshBuffer,
) where
    H: Height,
{
    output.reset(height_map.extent().num_points());

    // Avoid accessing out of bounds with a 3x3x3 kernel.
    let interior_extent = extent.add_to_shape(PointN([-2; 2])) + PointN([1; 2]);
    let interior_max = Extent2i::max(&interior_extent);

    let deltas = Point2i::basis();
    let mut delta_strides = [Stride(0); 2];
    height_map.strides_from_points(&deltas, &mut delta_strides);

    height_map.for_each_ref(
        &interior_extent,
        |(p, stride): (Point2i, Stride), height| {
            // Note: Although we use (x, y) for the coordinates of the height map, these should be
            // considered (x, z) in world coordinates, because +Y is the UP vector.
            let pz = p.y();
            let y = height.height();

            output.stride_to_index[stride.0] = output.mesh.positions.len();
            output.mesh.positions.push([p.x() as f32, y, pz as f32]);

            // Use central differencing to calculate the surface normal.
            //
            // From calculus, we know that gradients are always orthogonal to a level set. The
            // surface approximated by the height map h(x, z) happens to be the 0 level set of the
            // function:
            //
            // f(x, y, z) = y - h(x, z)
            //
            // And the gradient is:
            //
            // grad f = [-dh/dx, 1, -dh/dz]
            let l_stride = stride - delta_strides[0];
            let r_stride = stride + delta_strides[0];
            let b_stride = stride - delta_strides[1];
            let t_stride = stride + delta_strides[1];
            let l_y = height_map.get_unchecked_ref_release(l_stride).height();
            let r_y = height_map.get_unchecked_ref_release(r_stride).height();
            let b_y = height_map.get_unchecked_ref_release(b_stride).height();
            let t_y = height_map.get_unchecked_ref_release(t_stride).height();
            let dy_dx = (r_y - l_y) / 2.0;
            let dy_dz = (t_y - b_y) / 2.0;
            // Not normalized, because that's done more efficiently on the GPU.
            output.mesh.normals.push([-dy_dx, 1.0, -dy_dz]);
        },
    );

    Array2::<H>::for_each_point_and_stride(
        height_map.extent(),
        &interior_extent,
        |p, bl_stride| {
            // Only add a quad when p is the bottom-left corner of a quad that fits in the interior.
            if p < interior_max {
                let br_stride = bl_stride + delta_strides[0];
                let tl_stride = bl_stride + delta_strides[1];
                let tr_stride = bl_stride + delta_strides[0] + delta_strides[1];

                let bl_index = output.stride_to_index[bl_stride.0];
                let br_index = output.stride_to_index[br_stride.0];
                let tl_index = output.stride_to_index[tl_stride.0];
                let tr_index = output.stride_to_index[tr_stride.0];

                // Counter-clockwise winding.
                output.mesh.indices.extend_from_slice(&[
                    bl_index, tr_index, tl_index, bl_index, br_index, tr_index,
                ]);
            }
        },
    );
}
