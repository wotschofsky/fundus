fn main() {
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=../../data/vault.sqlite");
    println!("cargo:rerun-if-env-changed=DATABASE_URL");
}
