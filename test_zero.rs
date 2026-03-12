fn main() { let v = vec![0, 0, 0, 0]; println!("{:?}", v.iter().enumerate().max_by_key(|&(_, &x)| x)); }
