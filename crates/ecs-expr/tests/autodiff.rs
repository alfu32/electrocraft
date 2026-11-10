//! Property tests: autodiff matches finite differences, and constant folding and
//! CSE do not change results compared with a naive tree walk.

use ecs_expr::{BinOp, Expr, Leaf, Resolved, Sym, TapeBuilder, UnOp, Workspace};
use proptest::prelude::*;

fn naive(e: &Expr, x: f64, y: f64) -> f64 {
    match e {
        Expr::Num(v) => *v,
        Expr::Name(n) => {
            if n == "x" {
                x
            } else {
                y
            }
        }
        Expr::Un(op, a) => {
            let a = naive(a, x, y);
            match op {
                UnOp::Neg => -a,
                UnOp::Sin => a.sin(),
                UnOp::Cos => a.cos(),
                UnOp::Tanh => a.tanh(),
                UnOp::Atan => a.atan(),
                UnOp::Exp => a.exp(),
                _ => unreachable!(),
            }
        }
        Expr::Bin(op, a, b) => {
            let (a, b) = (naive(a, x, y), naive(b, x, y));
            match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => a / b,
                _ => unreachable!(),
            }
        }
        _ => unreachable!(),
    }
}

fn arb_expr() -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
        (-3.0f64..3.0).prop_map(Expr::Num),
        Just(Expr::name("x")),
        Just(Expr::name("y")),
    ];
    leaf.prop_recursive(5, 40, 2, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a + b),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a - b),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a * b),
            // Denominator kept away from zero.
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| a / (Expr::Num(2.0) + Expr::un(UnOp::Sin, b))),
            inner.clone().prop_map(|a| Expr::un(UnOp::Sin, a)),
            inner.clone().prop_map(|a| Expr::un(UnOp::Cos, a)),
            inner.clone().prop_map(|a| Expr::un(UnOp::Tanh, a)),
            inner.clone().prop_map(|a| Expr::un(UnOp::Atan, a)),
            inner
                .clone()
                .prop_map(|a| Expr::un(UnOp::Exp, Expr::un(UnOp::Tanh, a))),
            inner.prop_map(|a| -a),
        ]
    })
}

fn compile(e: &Expr) -> ecs_expr::Tape {
    let mut b = TapeBuilder::new(2);
    let id = b
        .add_expr(e, &mut |s| match s {
            Sym::Name("x") => Ok(Resolved::Leaf(Leaf::Unknown(0))),
            Sym::Name("y") => Ok(Resolved::Leaf(Leaf::Unknown(1))),
            _ => Err("unknown".into()),
        })
        .unwrap();
    b.finish(vec![id])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn autodiff_matches_finite_differences(e in arb_expr(), x in -2.0f64..2.0, y in -2.0f64..2.0) {
        let tape = compile(&e);
        let mut ws = Workspace::default();
        tape.eval(&[x, y], &[], &mut ws);
        let value = tape.value(&ws, 0);
        let grad = tape.grad(&ws, 0).to_vec();
        let reference = naive(&e, x, y);
        prop_assert!((value - reference).abs() <= 1e-9 * (1.0 + reference.abs()),
            "value {value} vs naive {reference}");
        let h = 1e-6;
        let dx = (naive(&e, x + h, y) - naive(&e, x - h, y)) / (2.0 * h);
        let dy = (naive(&e, x, y + h) - naive(&e, x, y - h)) / (2.0 * h);
        let scale = 1.0 + dx.abs().max(dy.abs());
        prop_assert!((grad[0] - dx).abs() <= 1e-4 * scale, "d/dx {} vs {dx}", grad[0]);
        prop_assert!((grad[1] - dy).abs() <= 1e-4 * scale, "d/dy {} vs {dy}", grad[1]);
    }
}
