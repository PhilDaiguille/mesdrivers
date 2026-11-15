use embed_manifest::{embed_manifest, manifest::ExecutionLevel, new_manifest};

fn main() {
    // Installer un pilote exige les droits admin : on déclenche l'invite UAC au lancement.
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest(new_manifest("MesDrivers").requested_execution_level(ExecutionLevel::RequireAdministrator))
            .expect("impossible d'embarquer le manifeste");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
