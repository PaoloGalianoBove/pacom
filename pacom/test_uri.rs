use up_rust::UUri;

fn main() {
    let uri = UUri::try_from_parts("*", 0, 0, 0).unwrap_or_default();
    println!("Auth name for '*': '{}'", uri.authority_name());
    let uri2 = UUri::default();
    println!("Auth name for default: '{}'", uri2.authority_name());
}
