extern crate basis;
extern crate fem_domain;
extern crate eigensolver;
extern crate integration;

pub use basis::{BasisFn, BasisFnSampler, KOLShapeFn, MaxOrthoShapeFn, ShapeFn};
pub use fem_domain::{Domain, Mesh, Point, M2D, V2D, DoF, HRef, PRef, HRefError, PRefError, BasisDir};
pub use integration::{CurlProduct, Integral, IntegralResult, L2InnerProduct, fill_matrices, fill_matrices_parallel};
pub use eigensolver::{GEP, SparseMatrix, solve_gep, EigenPair};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_correctness() {
        let i_max: usize = 2;
        let j_max: usize = 2;
        let p_elem_id = 1;
        let q_elem_id = 5;

        let mut se_mesh = Mesh::from_file("./test_input/test_mesh_c.json").unwrap();
        se_mesh.set_expansion_on_elems(vec![0], [i_max as u8, j_max as u8]).unwrap();
        se_mesh.h_refine_elems(vec![0], HRef::T).unwrap();
        se_mesh.h_refine_elems(vec![1], HRef::U(None)).unwrap();


        let (mut bs_sampler, [u_weights, v_weights]): (BasisFnSampler<KOLShapeFn>, _) =
            BasisFnSampler::with(i_max, j_max, None, None, false);

        let a_integ = CurlProduct::with_weights(&u_weights, &v_weights);
        let b_integ = L2InnerProduct::with_weights(&u_weights, &v_weights);

        let bs_p = bs_sampler.sample_basis_fn(&se_mesh.elems[p_elem_id], match q_elem_id == p_elem_id {
            false => Some(&se_mesh.elems[q_elem_id]),
            true => None,
        });
        let bs_q = bs_sampler.sample_basis_fn(&se_mesh.elems[q_elem_id], None);

        for p_dir in [BasisDir::U, BasisDir::V] {
            for q_dir in [BasisDir::U, BasisDir::V] {
                for p_i in 0..=i_max {
                    for p_j in 0..=j_max {
                        for q_i in 0..=i_max {
                            for q_j in 0..=j_max {
                                
                                let a = a_integ
                                    .integrate(p_dir, q_dir, [p_i, p_j], [q_i, q_j], &bs_p, &bs_q)
                                    .full_solution();
                                let b = b_integ
                                    .integrate(p_dir, q_dir, [p_i, p_j], [q_i, q_j], &bs_p, &bs_q)
                                    .full_solution();

                                println!(
                                    "[{}, ({}, {})] \t [{}, ({}, {})] \t {:.10} \t {:.10}",
                                    p_dir,
                                    p_i,
                                    p_j,
                                    q_dir,
                                    q_i,
                                    q_j,
                                    a,
                                    b
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn basic_problem() {
        let mut domain = Domain::from_mesh_file("./test_input/test_mesh_b.json").unwrap();

        domain.mesh.global_p_refinement(PRef::from(1, 1)).unwrap();
        // domain.mesh.global_h_refinement(HRef::T).unwrap();
        domain.mesh.h_refine_elems(vec![0], HRef::T).unwrap();
        domain.gen_dofs();

        println!("Num DoFs: {}", domain.dofs.len());
        for dof in domain.dofs.iter() {
            println!("{}", dof);
        }

        for bs in domain.basis_specs.iter().flatten() {
            println!("{}", bs);
        }

        let eigenproblem = fill_matrices::<CurlProduct, L2InnerProduct, KOLShapeFn>(&domain);
        let eigen_pair = solve_gep(eigenproblem, 1.475).unwrap();

        println!("Solution: {:.10}", eigen_pair.value);
    }
}
