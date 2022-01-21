mod glq;
mod kol;
mod max_ortho;

use fem_domain::{Elem, M2D, V2D};
use glq::{gauss_quadrature_points, scale_gauss_quad_points};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

pub use kol::KOLShapeFn;
pub use max_ortho::MaxOrthoShapeFn;

/// Hierarchical Shape Function along a single direction (defined over (-1.0, +1.0)).
/// [KOLShapeFn] and [MaxOrthoShapeFn] implement this trait.
/// Alternate Hierarchical Basis Functions can be used by implementing this trait.
pub trait ShapeFn: Clone + Sync + Send + std::fmt::Debug {
    fn with(max_order: usize, points: &[f64], compute_d2: bool) -> Self;

    fn power(&self, n: usize, p: usize) -> f64;
    fn power_d1(&self, n: usize, p: usize) -> f64;
    fn power_d2(&self, n: usize, p: usize) -> f64;

    fn poly(&self, n: usize, p: usize) -> f64;
    fn poly_d1(&self, n: usize, p: usize) -> f64;
    fn poly_d2(&self, n: usize, p: usize) -> f64;
}

/// Structure used to generate and cache [BasisFn]'s over [Elem]'s in a Domain.
#[derive(Debug)]
pub struct BasisFnSampler<SF: ShapeFn> {
    /// Maximum u-directed expansion order. [BasisFn]s Generated by this sampler will be defined up to this order in `i`
    pub i_max: usize,
    /// Maximum v-directed expansion order. [BasisFn]s Generated by this sampler will be defined up to this order in `i`              
    pub j_max: usize,
    /// Whether the 2nd derivatives of the [ShapeFn]s will be computed. If true, the endpoints [-1, +1] are also defined along both axes!                 
    pub compute_d2: bool,
    /// Gauss Legendre Quadrature points evaluated along u-direction. Defined from (-1 to +1)         
    u_points: Vec<f64>,
    /// Gauss Legendre Quadrature points evaluated along v-direction. Defined from (-1 to +1)
    v_points: Vec<f64>,

    computed: HashMap<BSDescription, Rc<BasisFn<SF>>>,
}

impl<SF: ShapeFn> BasisFnSampler<SF> {
    /// Construct a sampler with the following parameters:
    ///
    /// * `i_max` : maximum u-directed expansion order
    /// * `j_max` : maximum v-directed expansion order
    /// * `num_u_points` : number of Gauss-Leg-Quad points to define along the u-axis. If `None`: a default value is used
    /// * `num_v_points` : number of Gauss-Leg-Quad points to define along the v-axis. If `None`: a default value is used
    /// * `compute_2nd_derivs`: whether or not to compute the second derivatives of the [ShapeFn]s.
    pub fn with(
        i_max: usize,
        j_max: usize,
        num_u_points: Option<usize>,
        num_v_points: Option<usize>,
        compute_2nd_derivs: bool,
    ) -> (Self, [Vec<f64>; 2]) {
        let (u_points, u_weights) = gauss_quadrature_points(
            num_u_points.unwrap_or(default_ngq(i_max)),
            compute_2nd_derivs,
        );
        let (v_points, v_weights) = gauss_quadrature_points(
            num_v_points.unwrap_or(default_ngq(j_max)),
            compute_2nd_derivs,
        );

        (
            Self {
                i_max,
                j_max,
                compute_d2: compute_2nd_derivs,
                u_points,
                v_points,
                computed: HashMap::new(),
            },
            [u_weights, v_weights],
        )
    }

    /// Generate or retrieve a [BasisFn] defined over an [Elem]. Can be defined over a subset of the Element.
    pub fn sample_basis_fn(
        &mut self,
        elem: &Elem,
        over_desc_elem: Option<&Elem>,
    ) -> Rc<BasisFn<SF>> {
        let desc = BSDescription {
            space: [elem.nodes[0], elem.nodes[1]],
            sample: match over_desc_elem {
                Some(desc_elem) => Some([desc_elem.nodes[0], desc_elem.nodes[1]]),
                None => None,
            },
        };

        if let Some(computed_bs) = self.computed.get(&desc) {
            computed_bs.clone()
        } else {
            let bs = BasisFn::with(
                self.i_max,
                self.j_max,
                self.compute_d2,
                &self.u_points,
                &self.v_points,
                elem,
                over_desc_elem,
            );
            self.computed.insert(desc.clone(), Rc::new(bs));
            self.computed.get(&desc).unwrap().clone()
        }
    }
}

/// Same as [BasisFnSampler], except threadsafe internal data structures are used
pub struct ParBasisFnSampler<SF: ShapeFn> {
    /// Maximum u-directed expansion order. [BasisFn]s Generated by this sampler will be defined up to this order in `i`
    pub i_max: usize,
    /// Maximum v-directed expansion order. [BasisFn]s Generated by this sampler will be defined up to this order in `i`              
    pub j_max: usize,
    /// Whether the 2nd derivatives of the [ShapeFn]s will be computed. If true, the endpoints [-1, +1] are also defined along both axes!                 
    pub compute_d2: bool,
    /// Gauss Legendre Quadrature points evaluated along u-direction. Defined from (-1 to +1)         
    u_points: Vec<f64>,
    /// Gauss Legendre Quadrature points evaluated along v-direction. Defined from (-1 to +1)
    v_points: Vec<f64>,

    computed: Arc<Mutex<HashMap<BSDescription, Arc<BasisFn<SF>>>>>,
}

impl<SF: ShapeFn> ParBasisFnSampler<SF> {
    /// Construct a threadsafe sampler with the following parameters:
    ///
    /// * `i_max` : maximum u-directed expansion order
    /// * `j_max` : maximum v-directed expansion order
    /// * `num_u_points` : number of Gauss-Leg-Quad points to define along the u-axis. If `None`: a default value is used
    /// * `num_v_points` : number of Gauss-Leg-Quad points to define along the v-axis. If `None`: a default value is used
    /// * `compute_2nd_derivs`: whether or not to compute the second derivatives of the [ShapeFn]s.
    pub fn with(
        i_max: usize,
        j_max: usize,
        num_u_points: Option<usize>,
        num_v_points: Option<usize>,
        compute_2nd_derivs: bool,
    ) -> (Self, [Vec<f64>; 2]) {
        let (u_points, u_weights) = gauss_quadrature_points(
            num_u_points.unwrap_or(default_ngq(i_max)),
            compute_2nd_derivs,
        );
        let (v_points, v_weights) = gauss_quadrature_points(
            num_v_points.unwrap_or(default_ngq(j_max)),
            compute_2nd_derivs,
        );

        (
            Self {
                i_max,
                j_max,
                compute_d2: compute_2nd_derivs,
                u_points,
                v_points,
                computed: Arc::new(Mutex::new(HashMap::new())),
            },
            [u_weights, v_weights],
        )
    }

    /// Generate or retrieve a [BasisFn] defined over an [Elem]. Can be defined over a subset of the Element.
    pub fn sample_basis_fn(
        &mut self,
        elem: &Elem,
        over_desc_elem: Option<&Elem>,
    ) -> Arc<BasisFn<SF>> {
        let desc = BSDescription {
            space: [elem.nodes[0], elem.nodes[1]],
            sample: match over_desc_elem {
                Some(desc_elem) => Some([desc_elem.nodes[0], desc_elem.nodes[1]]),
                None => None,
            },
        };

        match self.computed.lock() {
            Ok(mut comp_guard) => {
                if let Some(computed_bs) = comp_guard.get(&desc) {
                    computed_bs.clone()
                } else {
                    let bs = BasisFn::with(
                        self.i_max,
                        self.j_max,
                        self.compute_d2,
                        &self.u_points,
                        &self.v_points,
                        elem,
                        over_desc_elem,
                    );
                    comp_guard.insert(desc.clone(), Arc::new(bs));
                    comp_guard.get(&desc).unwrap().clone()
                }
            }
            // fallback on computing directly, if MutexGuard is not available.
            Err(_) => Arc::new(BasisFn::with(
                self.i_max,
                self.j_max,
                self.compute_d2,
                &self.u_points,
                &self.v_points,
                elem,
                over_desc_elem,
            )),
        }
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
// unique description of a basis sample. 
// TODO: update this struct to be more robust to curvilinear Elements
struct BSDescription {
    space: [usize; 2],
    sample: Option<[usize; 2]>,
}

// 5 * the maximum order (rounded up to the nearest power of 2)
fn default_ngq(max_order: usize) -> usize {
    let conv = (max_order * 5) as f32;
    let conv_p2 = conv.log2().ceil() as i32;

    (2.0_f32).powi(conv_p2).round() as usize
}

/// Structure used to evaluate [ShapeFn]'s and their derivatives over some area
#[derive(Clone, Debug)]
pub struct BasisFn<SF: ShapeFn> {
    /// Raw transformation matrices at each sample point. Describes transformation from real space to sampled parametric space.
    pub t: Vec<Vec<M2D>>,
    // Inverse of transformation matrices at each sample point.
    pub ti: Vec<Vec<M2D>>,
    /// Determinants of the "Sampling Jacobian" at each point.
    pub dt: Vec<Vec<f64>>,
    /// Parametric scaling factors (used to scale derivatives in parametric space as necessary)
    pub para_scale: V2D,
    u_shapes: SF,
    v_shapes: SF,
}

impl<SF: ShapeFn> BasisFn<SF> {
    pub fn with(
        i_max: usize,
        j_max: usize,
        compute_d2: bool,
        raw_u_points: &[f64],
        raw_v_points: &[f64],
        elem: &Elem,
        over_child_elem: Option<&Elem>,
    ) -> Self {
        let [
            (u_glq_scale, u_points_scaled), 
            (v_glq_scale, v_points_scaled),
         ] = match over_child_elem {
            Some(child_elem) => {
                let child_parametric_range = child_elem.relative_parametric_range(elem.id);
                [
                    scale_gauss_quad_points(raw_u_points, child_parametric_range[0][0], child_parametric_range[0][1]),
                    scale_gauss_quad_points(raw_v_points, child_parametric_range[1][0], child_parametric_range[1][1]),
                ]
            }
            None => [
                (1.0, raw_u_points.to_vec()), 
                (1.0, raw_v_points.to_vec()),
            ],
        };

        let t: Vec<Vec<M2D>> = u_points_scaled
            .iter()
            .map(|u| {
                v_points_scaled
                    .iter()
                    .map(|v| elem.parametric_mapping(
                        V2D::from([*u, *v]),
                        elem.parametric_range(),
                    ))
                    .collect()
            })
            .collect();

        let ti: Vec<Vec<M2D>> = t
            .iter()
            .map(|row| row.iter().map(|v| v.inverse()).collect())
            .collect();

        // let dt = if sampled_space.is_some() {
        //     t.iter()
        //         .map(|row| row.iter().map(|v| v.det()).collect())
        //         .collect()
        // } else {
        //     vec![vec![1.0; raw_v_points.len()]; raw_u_points.len()]
        // };

        let dt: Vec<Vec<f64>> = t.iter()
            .map(|row| row.iter().map(|v| v.det()).collect())
            .collect();

        println!("t: {} \t ti: {} \t dt: {} \t UScale: {} \t VScale: {}", t[0][0], ti[0][0], dt[0][0], v_glq_scale, u_glq_scale);

        Self {
            t,
            ti,
            dt,
            para_scale: V2D::from([v_glq_scale, u_glq_scale]),
            u_shapes: SF::with(i_max, &u_points_scaled, compute_d2),
            v_shapes: SF::with(j_max, &v_points_scaled, compute_d2),
        }
    }

    pub fn f_u(&self, [i, j]: [usize; 2], [m, n]: [usize; 2]) -> V2D {
        self.ti[m][n].u * self.u_shapes.power(i, m) * self.v_shapes.poly(j, n)
    }

    pub fn f_v(&self, [i, j]: [usize; 2], [m, n]: [usize; 2]) -> V2D {
        self.ti[m][n].v * self.u_shapes.poly(i, m) * self.v_shapes.power(j, n)
    }

    pub fn f_u_d1(&self, [i, j]: [usize; 2], [m, n]: [usize; 2], para_scale: &V2D) -> V2D {
        self.ti[m][n].u
            * V2D::from([
                self.u_shapes.power(i, m) * self.v_shapes.poly_d1(j, n),
                self.u_shapes.power_d1(i, m) * self.v_shapes.poly(j, n),
            ])
            * para_scale
    }

    pub fn f_v_d1(&self, [i, j]: [usize; 2], [m, n]: [usize; 2], para_scale: &V2D) -> V2D {
        self.ti[m][n].v
            * V2D::from([
                self.u_shapes.poly(i, m) * self.v_shapes.power_d1(j, n),
                self.u_shapes.poly_d1(i, m) * self.v_shapes.power(j, n),
            ])
            * para_scale
    }

    pub fn f_u_d2(&self, [i, j]: [usize; 2], [m, n]: [usize; 2], para_scale: &V2D) -> V2D {
        self.ti[m][n].u
            * V2D::from([
                self.u_shapes.power(i, m) * self.v_shapes.poly_d2(j, n),
                self.u_shapes.power_d2(i, m) * self.v_shapes.poly(j, n),
            ])
            * para_scale
            * para_scale
    }

    pub fn f_v_d2(&self, [i, j]: [usize; 2], [m, n]: [usize; 2], para_scale: &V2D) -> V2D {
        self.ti[m][n].v
            * V2D::from([
                self.u_shapes.poly(i, m) * self.v_shapes.power_d2(j, n),
                self.u_shapes.poly_d2(i, m) * self.v_shapes.power(j, n),
            ])
            * para_scale
            * para_scale
    }

    pub fn f_u_dd(&self, [i, j]: [usize; 2], [m, n]: [usize; 2], para_scale: &V2D) -> V2D {
        self.ti[m][n].u
            * self.u_shapes.power_d1(i, m)
            * self.v_shapes.poly_d1(j, n)
            * para_scale[0]
            * self.para_scale[1]
    }

    pub fn f_v_dd(&self, [i, j]: [usize; 2], [m, n]: [usize; 2], para_scale: &V2D) -> V2D {
        self.ti[m][n].v
            * self.u_shapes.poly_d1(i, m)
            * self.v_shapes.power_d1(j, n)
            * para_scale[0]
            * self.para_scale[1]
    }

    #[inline]
    pub fn glq_scale(&self) -> f64 {
        self.para_scale[0] * self.para_scale[1]
    }

    #[inline]
    pub fn edge_glq_scale(&self, edge_idx: usize) -> f64 {
        match edge_idx {
            0 | 1 => self.para_scale[1],
            2 | 3 => self.para_scale[0],
            _ => panic!("edge_idx must not exceed 3; cannot get glq scaling factor!"),
        }
    }

    #[inline]
    pub fn u_glq_scale(&self) -> f64 {
        self.para_scale[1]
    }

    #[inline]
    pub fn v_glq_scale(&self) -> f64 {
        self.para_scale[0]
    }

    #[inline]
    pub fn deriv_scale(&self) -> &V2D {
        &self.para_scale
    }

    #[inline]
    pub fn sample_scale(&self, [m, n]: [usize; 2]) -> f64 {
        self.dt[m][n]
    }
}
