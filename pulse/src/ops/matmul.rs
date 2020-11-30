use crate::internal::*;
use tract_core::ops::matmul::MatMulUnary;

submit_op_pulsifier!(MatMulUnary, pulsify);

fn pulsify(
    op: &MatMulUnary,
    _source: &TypedModel,
    node: &TypedNode,
    target: &mut PulsedModel,
    mapping: &HashMap<OutletId, OutletId>,
    _pulse: usize,
) -> TractResult<TVec<OutletId>> {
    let input = mapping[&node.inputs[0]];
    let fact = target.outlet_fact(input)?;
    if fact.axis >= fact.shape.len() - op.b_trans as usize {
        bail!("Can not pulsify MatMulUnaryA on the k dimension");
    }
    target.wire_node(&*node.name, op.clone(), &[input])
}
impl PulsedOp for MatMulUnary {
    fn pulsed_output_facts(&self, inputs: &[&PulsedFact]) -> TractResult<TVec<PulsedFact>> {
        let mut fact = inputs[0].clone();
        let (_m, _k, _n, c_shape) = tract_core::ops::matmul::compute_shape(
            &self.a.shape().into_iter().map(|d| d.to_dim()).collect::<TVec<_>>(),
            &inputs[0].shape,
            self.a_trans,
            self.b_trans,
            self.c_trans,
        )?;
        fact.datum_type = tract_core::ops::matmul::output_type(inputs[0].datum_type);
        fact.shape = c_shape;
        Ok(tvec!(fact))
    }

    as_op!();
    pulsed_op_to_typed_op!();
}
