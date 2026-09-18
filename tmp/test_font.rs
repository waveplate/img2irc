use std::collections::HashMap;

fn main() {
    let factor_x = 10.0;
    let factor_y = 20.0;
    let wc = 80.0;
    let orig_w = 100.0;
    let orig_h = 112.0;

    let pw = wc * factor_x;
    let ph = pw * (orig_h / orig_w);
    let rc = ph / factor_y;

    println!("Cols: {}, Rows: {}", wc, rc);
    println!("PNG AR: {}", pw / ph);
    println!("Term AR (assuming 1:2 char): {}", (wc * 1.0) / (rc * 2.0));
}
