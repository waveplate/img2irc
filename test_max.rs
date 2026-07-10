fn main() { let v = vec![1, 2, 2, 1]; println!("{:?}", v.iter().enumerate().max_by_key(|&(_, &x)| x)); }
