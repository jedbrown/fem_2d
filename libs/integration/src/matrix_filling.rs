use super::Integral;
use basis::{BasisFnSampler, ParBasisFnSampler, ShapeFn};
use fem_domain::Domain;
use sparse_matrix::SparseMatrix;

use rayon::prelude::*;
use std::mem;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

/// Fill two system matrices with the integrals of each pair of overlapping shape functions within a [Domain]
///
/// * Two [Integral]s: `AI` and `BI` must be specified. Their results are returned in the first and second [SparseMatrix]  
/// * A [ShapeFn] `SF` must also be specified
pub fn fill_matrices<AI, BI, SF>(domain: &Domain) -> [SparseMatrix; 2]
where
    AI: Integral,
    BI: Integral,
    SF: ShapeFn,
{
    // construct sparse system matrices
    let mut a_mat = SparseMatrix::new(domain.dofs.len());
    let mut b_mat = SparseMatrix::new(domain.dofs.len());

    // construct basis sampler
    let [i_max, j_max] = domain.mesh.max_expansion_orders();
    let (mut bs_sampler, [u_weights, v_weights]): (BasisFnSampler<SF>, _) =
        BasisFnSampler::with(i_max as usize, j_max as usize, None, None, false);

    // setup integration
    let a_integrator = AI::with_weights(&u_weights, &v_weights);
    let b_integrator = BI::with_weights(&u_weights, &v_weights);

    for elem in domain.elems() {
        // get relevant data for this Elem
        let bs_local = bs_sampler.sample_basis_fn(elem, None);
        let local_basis_specs = domain.local_basis_specs(elem.id).unwrap();
        let desc_basis_specs = domain.descendant_basis_specs(elem.id).unwrap();

        // local - local
        for (i, (p_orders, p_dir, p_dof_id)) in local_basis_specs
            .iter()
            .map(|bs_p| bs_p.get_integration_data())
            .enumerate()
        {
            for (q_orders, q_dir, q_dof_id) in local_basis_specs
                .iter()
                .skip(i)
                .map(|bs_q| bs_q.get_integration_data())
            {
                let a = a_integrator
                    .integrate(p_dir, q_dir, p_orders, q_orders, &bs_local, &bs_local)
                    .surface();
                let b = b_integrator
                    .integrate(p_dir, q_dir, p_orders, q_orders, &bs_local, &bs_local)
                    .surface();

                a_mat.insert([p_dof_id, q_dof_id], a);
                b_mat.insert([p_dof_id, q_dof_id], b);
            }
        }

        // local - desc
        for (p_orders, p_dir, p_dof_id) in local_basis_specs
            .iter()
            .map(|bs_p| bs_p.get_integration_data())
        {
            for &(q_elem_id, q_elem_basis_specs) in desc_basis_specs.iter() {
                let bs_p_sampled =
                    bs_sampler.sample_basis_fn(elem, Some(domain.mesh.elem_diag_points(q_elem_id)));
                let bs_q_local = bs_sampler.sample_basis_fn(&domain.mesh.elems[q_elem_id], None);

                for (q_orders, q_dir, q_dof_id) in q_elem_basis_specs
                    .iter()
                    .map(|bs_q| bs_q.get_integration_data())
                {
                    let a = a_integrator
                        .integrate(p_dir, q_dir, p_orders, q_orders, &bs_p_sampled, &bs_q_local)
                        .surface();
                    let b = b_integrator
                        .integrate(p_dir, q_dir, p_orders, q_orders, &bs_p_sampled, &bs_q_local)
                        .surface();

                    a_mat.insert([p_dof_id, q_dof_id], a);
                    b_mat.insert([p_dof_id, q_dof_id], b);
                }
            }
        }
    }

    [a_mat, b_mat]
}

/// Same as [fill_matrices], except integration is done in parallel using the global Rayon ThreadPool
pub fn fill_matrices_parallel<AI, BI, SF>(domain: &Domain) -> [SparseMatrix; 2]
where
    AI: Integral,
    BI: Integral,
    SF: ShapeFn,
{
    // construct sparse system matrices in a matrix collector which can collect and combine sub-matrices from Rayon threads
    let mut matrix_collector = DoubleMatrixParCollector::new(domain.dofs.len());

    // construct basis sampler
    let [i_max, j_max] = domain.mesh.max_expansion_orders();
    let (bs_sampler, [u_weights, v_weights]): (ParBasisFnSampler<SF>, _) =
        ParBasisFnSampler::with(i_max as usize, j_max as usize, None, None, false);
    let bs_sampler_send = Arc::new(Mutex::new(bs_sampler));

    // setup integration
    let a_integrator = AI::with_weights(&u_weights, &v_weights);
    let b_integrator = BI::with_weights(&u_weights, &v_weights);

    matrix_collector.par_extend(domain.mesh.elems.par_iter().map(|elem| {
        let mut local_a = SparseMatrix::new(domain.dofs.len());
        let mut local_b = SparseMatrix::new(domain.dofs.len());

        let bs_sampler_elem = bs_sampler_send.clone();

        // get relevant data for this Elem
        let bs_local = bs_sampler_elem.lock().unwrap().sample_basis_fn(elem, None);
        let local_basis_specs = domain.local_basis_specs(elem.id).unwrap();
        let desc_basis_specs = domain.descendant_basis_specs(elem.id).unwrap();

        // local - local
        for (i, (p_orders, p_dir, p_dof_id)) in local_basis_specs
            .iter()
            .map(|bs_p| bs_p.get_integration_data())
            .enumerate()
        {
            for (q_orders, q_dir, q_dof_id) in local_basis_specs
                .iter()
                .skip(i)
                .map(|bs_q| bs_q.get_integration_data())
            {
                let a = a_integrator
                    .integrate(p_dir, q_dir, p_orders, q_orders, &bs_local, &bs_local)
                    .surface();
                let b = b_integrator
                    .integrate(p_dir, q_dir, p_orders, q_orders, &bs_local, &bs_local)
                    .surface();

                local_a.insert([p_dof_id, q_dof_id], a);
                local_b.insert([p_dof_id, q_dof_id], b);
            }
        }

        // local - desc
        for (p_orders, p_dir, p_dof_id) in local_basis_specs
            .iter()
            .map(|bs_p| bs_p.get_integration_data())
        {
            for &(q_elem_id, q_elem_basis_specs) in desc_basis_specs.iter() {
                let bs_p_sampled = bs_sampler_elem
                    .lock()
                    .unwrap()
                    .sample_basis_fn(elem, Some(domain.mesh.elem_diag_points(q_elem_id)));
                let bs_q_local = bs_sampler_elem
                    .lock()
                    .unwrap()
                    .sample_basis_fn(&domain.mesh.elems[q_elem_id], None);

                for (q_orders, q_dir, q_dof_id) in q_elem_basis_specs
                    .iter()
                    .map(|bs_q| bs_q.get_integration_data())
                {
                    let a = a_integrator
                        .integrate(p_dir, q_dir, p_orders, q_orders, &bs_p_sampled, &bs_q_local)
                        .surface();
                    let b = b_integrator
                        .integrate(p_dir, q_dir, p_orders, q_orders, &bs_p_sampled, &bs_q_local)
                        .surface();

                    local_a.insert([p_dof_id, q_dof_id], a);
                    local_b.insert([p_dof_id, q_dof_id], b);
                }
            }
        }

        [local_a, local_b]
    }));

    matrix_collector.yield_matrices()
}

struct DoubleMatrixParCollector {
    a: SparseMatrix,
    b: SparseMatrix,
}

impl DoubleMatrixParCollector {
    pub fn new(dimension: usize) -> Self {
        Self {
            a: SparseMatrix::new(dimension),
            b: SparseMatrix::new(dimension),
        }
    }

    pub fn yield_matrices(self) -> [SparseMatrix; 2] {
        [self.a, self.b]
    }
}

impl ParallelExtend<[SparseMatrix; 2]> for DoubleMatrixParCollector {
    fn par_extend<I>(&mut self, elem_matrices_iter: I)
    where
        I: IntoParallelIterator<Item = [SparseMatrix; 2]>,
    {
        let (sender, receiver) = channel();

        elem_matrices_iter
            .into_par_iter()
            .for_each_with(sender, |s, mut elem_matrices| {
                s.send([
                    mem::replace(&mut elem_matrices[0], SparseMatrix::new(0)),
                    mem::replace(&mut elem_matrices[0], SparseMatrix::new(0)),
                ])
                .expect("Failed to send sub-matrices over MSPC channel; cannot construct Matrices!")
            });

        receiver
            .iter()
            .for_each(|[mut elem_a_mat, mut elem_b_mat]| {
                self.a.consume_matrix(&mut elem_a_mat);
                self.b.consume_matrix(&mut elem_b_mat);
            });
    }
}
