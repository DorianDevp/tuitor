fn total(prices: &[u32]) -> u32 {
    let mut sum = 0;
    for price in prices {
        sum += price;
    }
    sum
}

fn main() {
    let prices = vec![10, 20, 30];
    let result = total(&prices);
    println!("total={result}");
}
