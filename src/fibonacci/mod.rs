use num_traits::One;

use crate::core::air::mask::{Mask, MaskItem};
use crate::core::circle::Coset;
use crate::core::constraints::{coset_vanishing, point_excluder, point_vanishing, PolyOracle};
use crate::core::fields::m31::BaseField;
use crate::core::fields::ExtensionOf;
use crate::core::poly::circle::{CanonicCoset, CircleDomain, CircleEvaluation};
use crate::examples::fibonacci_code;

pub mod component;

pub struct Fibonacci {
    pub trace_coset: CanonicCoset,
    pub eval_domain: CircleDomain,
    pub constraint_coset: Coset,
    pub constraint_eval_domain: CircleDomain,
    pub claim: BaseField,
}

impl Fibonacci {
    pub fn new(n_bits: usize, claim: BaseField) -> Self {
        let trace_coset = CanonicCoset::new(n_bits);
        let eval_domain = trace_coset.evaluation_domain(n_bits + 1);
        let constraint_coset = Coset::subgroup(n_bits);
        let constraint_eval_domain = CircleDomain::constraint_evaluation_domain(n_bits + 1);
        Self {
            trace_coset,
            eval_domain,
            constraint_coset,
            constraint_eval_domain,
            claim,
        }
    }

    pub fn get_trace(&self) -> CircleEvaluation<BaseField> {
        // Fill trace with fibonacci squared.
        // TODO(spapini): Extract correct component instance.
        assert!(self.trace_coset.size() == 32);
        let trace = fibonacci_code::compute(&fibonacci_code::Input {
            secret: vec![BaseField::one()],
        });

        // Returns as a CircleEvaluation.
        CircleEvaluation::new_canonical_ordered(self.trace_coset, trace.f)
    }

    pub fn eval_step_constraint<F: ExtensionOf<BaseField>>(&self, trace: impl PolyOracle<F>) -> F {
        trace.get_at(self.trace_coset.index_at(0)).square()
            + trace.get_at(self.trace_coset.index_at(1)).square()
            - trace.get_at(self.trace_coset.index_at(2))
    }

    pub fn eval_step_quotient<F: ExtensionOf<BaseField>>(&self, trace: impl PolyOracle<F>) -> F {
        let excluded0 = self.constraint_coset.at(self.constraint_coset.size() - 2);
        let excluded1 = self.constraint_coset.at(self.constraint_coset.size() - 1);
        let num = self.eval_step_constraint(trace)
            * point_excluder(excluded0, trace.point())
            * point_excluder(excluded1, trace.point());
        let denom = coset_vanishing(self.constraint_coset, trace.point());
        num / denom
    }

    pub fn eval_boundary_constraint<F: ExtensionOf<BaseField>>(
        &self,
        trace: impl PolyOracle<F>,
        value: BaseField,
    ) -> F {
        trace.get_at(self.trace_coset.index_at(0)) - value
    }

    pub fn eval_boundary_quotient<F: ExtensionOf<BaseField>>(
        &self,
        trace: impl PolyOracle<F>,
        point_index: usize,
        value: BaseField,
    ) -> F {
        let num = self.eval_boundary_constraint(trace, value);
        let denom = point_vanishing(self.constraint_coset.at(point_index), trace.point());
        num / denom
    }

    pub fn eval_quotient<F: ExtensionOf<BaseField>, EF: ExtensionOf<F>>(
        &self,
        random_coeff: EF,
        trace: impl PolyOracle<F>,
    ) -> EF {
        let mut quotient = random_coeff.pow(0) * self.eval_step_quotient(trace);
        quotient += random_coeff.pow(1) * self.eval_boundary_quotient(trace, 0, BaseField::one());
        quotient += random_coeff.pow(2)
            * self.eval_boundary_quotient(trace, self.constraint_coset.size() - 1, self.claim);
        quotient
    }

    pub fn get_mask(&self) -> Mask {
        Mask::new(
            (0..3)
                .map(|offset| MaskItem {
                    column_index: 0,
                    offset,
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use num_traits::One;

    use super::Fibonacci;
    use crate::core::circle::CirclePoint;
    use crate::core::constraints::{EvalByEvaluation, EvalByPoly};
    use crate::core::fields::m31::{BaseField, M31};
    use crate::core::fields::qm31::QM31;
    use crate::core::poly::circle::CircleEvaluation;
    use crate::{m31, qm31};

    #[test]
    fn test_constraint_on_trace() {
        use num_traits::Zero;

        let fib = Fibonacci::new(5, m31!(443693538));
        let trace = fib.get_trace();

        // Assert that the step constraint is satisfied on the trace.
        for p_ind in fib
            .constraint_coset
            .iter_indices()
            .take(fib.constraint_coset.size() - 2)
        {
            let res = fib.eval_step_constraint(EvalByEvaluation {
                offset: p_ind,
                eval: &trace,
            });
            assert_eq!(res, BaseField::zero());
        }

        // Assert that the first trace value is 1.
        assert_eq!(
            fib.eval_boundary_constraint(
                EvalByEvaluation {
                    offset: fib.constraint_coset.index_at(0),
                    eval: &trace,
                },
                BaseField::one()
            ),
            BaseField::zero()
        );

        // Assert that the last trace value is the fibonacci claim.
        assert_eq!(
            fib.eval_boundary_constraint(
                EvalByEvaluation {
                    offset: fib
                        .constraint_coset
                        .index_at(fib.constraint_coset.size() - 1),
                    eval: &trace,
                },
                fib.claim
            ),
            BaseField::zero()
        );
    }

    #[test]
    fn test_quotient_is_low_degree() {
        let fib = Fibonacci::new(5, m31!(443693538));
        let trace = fib.get_trace();
        let trace_poly = trace.interpolate();

        let extended_evaluation = trace_poly.clone().evaluate(fib.eval_domain);

        // TODO(ShaharS), Change to a channel implementation to retrieve the random
        // coefficients from extension field.
        let random_coeff = qm31!(2213980, 2213981, 2213982, 2213983);

        // Compute quotient on the evaluation domain.
        let mut quotient_values = Vec::with_capacity(fib.constraint_eval_domain.size());
        for p_ind in fib.constraint_eval_domain.iter_indices() {
            quotient_values.push(fib.eval_quotient(
                random_coeff,
                EvalByEvaluation {
                    offset: p_ind,
                    eval: &extended_evaluation,
                },
            ));
        }
        let quotient_eval = CircleEvaluation::new(fib.constraint_eval_domain, quotient_values);
        // Interpolate the poly. The poly is indeed of degree lower than the size of
        // eval_domain, then it should interpolate correctly.
        let interpolated_quotient_poly = quotient_eval.interpolate();

        // Evaluate this polynomial at another point, out of eval_domain and compare to what we
        // expect.
        let oods_point = CirclePoint::<QM31>::get_point(98989892);
        let trace_evaluator = EvalByPoly {
            point: oods_point,
            poly: &trace_poly,
        };

        assert_eq!(
            interpolated_quotient_poly.eval_at_point(oods_point),
            fib.eval_quotient(random_coeff, trace_evaluator)
        );
    }
}