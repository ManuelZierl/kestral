fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let result = match arguments.as_slice() {
        [owner, pair] if owner == "owner" && pair == "pair" => {
            host_lib::remote_api::create_owner_pairing_code_from_env()
        }
        [owner, pair, ..] if owner == "owner" && pair == "pair" => {
            host_lib::remote_api::create_owner_pairing_code_from_env()
        }
        [owner, reset, rest @ ..] if owner == "owner" && reset == "reset" => {
            if rest.iter().any(|argument| argument == "--confirm") {
                host_lib::remote_api::reset_owner_authentication_from_env()
            } else {
                Err("owner reset requires --confirm and the backend must be stopped".into())
            }
        }
        _ => host_lib::remote_api::run_from_env(),
    };
    if let Err(error) = result {
        eprintln!("backend-only host failed: {error}");
        std::process::exit(1);
    }
}
