fn renamed(input: u8) -> u8 {
    let value = None::<u8>;
    { let value = Some(input); if input == 7 { return value.unwrap(); } }
    if input == 8 { value.unwrap() } else { 0 }
}
