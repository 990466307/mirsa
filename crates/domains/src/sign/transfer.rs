use rustc_middle::mir::*;
use rustc_middle::ty::{TyCtxt, TyKind};

use super::abstract_value::*;
use super::state::SignState;

fn signed_to_i128(bits: u128, bw: u64) -> i128 {

    if bw == 128 {
        return bits as i128;
    }

    let sign_bit = 1u128 << (bw - 1);
    let mask = (1u128 << bw) - 1;
    let x = bits & mask;

    if (x & sign_bit) != 0 {
        (x as i128) - ((1u128 << bw) as i128)
    } else {
        x as i128
    }
}

fn sign_of_const<'tcx>(c: &ConstOperand<'tcx>) -> Sign {
    let ty = c.ty();

    let (is_signed, _is_int) = match ty.kind() {
        TyKind::Int(_) => (true, true),
        TyKind::Uint(_) => (false, true),
        _ => return Sign::Top, // 不是整数：不在本域内
    };
    let k = c.const_;
    if let Some(si) = k.try_to_scalar_int() {
        let bw = si.size().bits();
        let bits = si.to_bits_unchecked();
        if is_signed {
            let v: i128 = signed_to_i128(bits, bw);
            if v == 0 {
                Sign::Zero
            } else if v > 0 {
                Sign::Pos
            } else {
                Sign::Neg
            }
        } else {
            if bits == 0 { Sign::Zero } else { Sign::Pos }
        }
    } else {
        Sign::Top
    }
}

fn eval_operand<'tcx>(_tcx: TyCtxt<'tcx>, op: &Operand<'tcx>, st: &SignState) -> Sign {
    match op {
        Operand::Copy(p) | Operand::Move(p) => {
            if let Some(l) = p.as_local() {
                st.get_local(l)
            } else {
                Sign::Top
            }
        }
        Operand::Constant(c) => sign_of_const(c),
    }
}

pub fn transfer_stmt<'tcx>(tcx: TyCtxt<'tcx>, st: &mut SignState, stmt: &Statement<'tcx>) {
    let StatementKind::Assign(assign) = &stmt.kind else {
        return;
    };
    let (place, rvalue) = &**assign;

    let Some(dst) = place.as_local() else {
        return;
    };

    let rhs_sign = match rvalue {
        Rvalue::Use(op) => eval_operand(tcx, op, st),

        Rvalue::UnaryOp(op, arg) => {
            let s = eval_operand(tcx, arg, st);
            match op {
                UnOp::Neg => neg(s),
                _ => Sign::Top,
            }
        }

        Rvalue::BinaryOp(op, ops) => {
            let (a, b) = &**ops;
            let sa = eval_operand(tcx, a, st);
            let sb = eval_operand(tcx, b, st);

            use rustc_middle::mir::BinOp::*;
            match op {
                Add => add(sa, sb),
                Sub => sub(sa, sb),
                Mul => mul(sa, sb),
                Div => div(sa, sb),
                Rem => Sign::Top, // 余数符号更复杂，先 Top
                _ => Sign::Top,
            }
        }

        _ => Sign::Top,
    };

    st.set_local(dst, rhs_sign);
}

pub fn transfer_block<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    bb: BasicBlock,
    in_state: &SignState,
) -> SignState {
    let mut st = in_state.clone();
    let data = &body.basic_blocks[bb];
    for stmt in &data.statements {
        transfer_stmt(tcx, &mut st, stmt);
    }
    st
}
