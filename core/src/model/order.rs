//! Evaluation order for nodes.
use crate::internal::*;
use bit_set;
use std::fmt::{Debug, Display};

/// Find an evaluation order for a model, using its default inputs and outputs
/// as boundaries.
pub fn eval_order<TI: TensorInfo, O: Debug + Display + AsRef<Op> + AsMut<Op>>(model: &super::Model<TI, O>) -> TractResult<Vec<usize>> {
    let inputs = model.input_outlets()?.iter().map(|n| n.node).collect::<Vec<usize>>();
    let targets = model.output_outlets()?.iter().map(|n| n.node).collect::<Vec<usize>>();
    eval_order_for_nodes(model.nodes(), &inputs, &targets)
}

/// Find a working evaluation order for a list of nodes.
pub fn eval_order_for_nodes<TI: TensorInfo, O:Debug + Display + AsRef<Op> + AsMut<Op>>(
    nodes: &[BaseNode<TI, O>],
    inputs: &[usize],
    targets: &[usize],
) -> TractResult<Vec<usize>> {
    let mut done = bit_set::BitSet::with_capacity(nodes.len());
    let mut needed: Vec<usize> = vec![];
    let mut order: Vec<usize> = vec![];
    for &t in targets {
        needed.push(t);
    }
    while let Some(&node) = needed.last() {
        if done.contains(node) {
            needed.pop();
            continue;
        }
        if inputs.contains(&node) || nodes[node].inputs.iter().all(|i| done.contains(i.node)) {
            order.push(node);
            needed.pop();
            done.insert(node);
        } else {
            for input in nodes[node].inputs.iter().rev() {
                if !done.contains(input.node) {
                    needed.push(input.node);
                }
            }
        }
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use crate::internal::*;
    use crate::ops::math::Add;

    #[test]
    fn test_simple() {
        let mut model = Model::default();
        model.add_source_default("a").unwrap();
        model.chain_default("add", Add::default()).unwrap();
        model.add_const("b", Tensor::from(12.0f32)).unwrap();
        model.add_edge(OutletId::new(2, 0), InletId::new(1, 1)).unwrap();
        assert_eq!(model.eval_order().unwrap(), vec!(0, 2, 1));
    }

    #[test]
    fn test_diamond() {
        let mut model = Model::default();
        model.add_source_default("a").unwrap();
        model.chain_default("add", Add::default()).unwrap();
        model.add_edge(OutletId::new(0, 0), InletId::new(1, 1)).unwrap();
        assert_eq!(model.eval_order().unwrap(), vec!(0, 1));
    }
}
