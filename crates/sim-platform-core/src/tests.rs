use super::*;

const CAPSULE: &str = r#"
schema = "sim.platform-capsule/v1"
provider = "platform/site/fictional"
services = ["service/fictional-clock"]
shells = ["shell/fictional"]
loader_kinds = ["loader/static-v1"]
"#;
const BUNDLE: &str = r#"
schema = "sim.platform-bundle/v1"
capsule = "platform/site/fictional"
artifact = "lib/sim-platform-fictional"
loader = "loader/static-v1"
artifact_content = "sha256:fictional"
entry = "sim_native_abi_v1"
shell = "shell/fictional"
"#;

fn context<'a>() -> ValidationContext<'a> {
    ValidationContext {
        owner_repository: "sim-platform",
        declaring_repository: "sim-platform",
        bootstrap_packages: &["sim-platform-bootstrap"],
        product_closure_roles: &[PackageRole::SimPure, PackageRole::PlatformBootstrap],
    }
}

#[test]
fn fictional_records_parse_and_validate() {
    let capsule = parse_capsule(CAPSULE).unwrap();
    validate_capsules(&context(), std::slice::from_ref(&capsule)).unwrap();
    validate_bundle(&parse_bundle(BUNDLE).unwrap(), &capsule).unwrap();
}

#[test]
fn absent_role_is_pure_and_all_roles_are_closed() {
    assert_eq!(PackageRole::parse(None), Ok(PackageRole::SimPure));
    for (name, role) in [
        ("sim-pure", PackageRole::SimPure),
        ("platform-bootstrap", PackageRole::PlatformBootstrap),
        ("platform-capsule", PackageRole::PlatformCapsule),
        ("host-tool", PackageRole::HostTool),
    ] {
        assert_eq!(PackageRole::parse(Some(name)), Ok(role));
    }
    assert!(matches!(
        PackageRole::parse(Some("magic")),
        Err(ValidationError::UnknownRole(_))
    ));
}

#[test]
fn rejects_capsule_outside_owner() {
    let mut ctx = context();
    ctx.declaring_repository = "sim-runtime";
    assert_eq!(
        validate_capsules(&ctx, &[parse_capsule(CAPSULE).unwrap()]),
        Err(ValidationError::CapsuleOutsideOwner)
    );
}

#[test]
fn rejects_duplicate_provider_ids() {
    let capsule = parse_capsule(CAPSULE).unwrap();
    assert!(matches!(
        validate_capsules(&context(), &[capsule.clone(), capsule]),
        Err(ValidationError::DuplicateProvider(_))
    ));
}

#[test]
fn rejects_undeclared_shell() {
    let capsule = parse_capsule(CAPSULE).unwrap();
    let mut bundle = parse_bundle(BUNDLE).unwrap();
    bundle.shell = Some("shell/other".into());
    assert!(matches!(
        validate_bundle(&bundle, &capsule),
        Err(ValidationError::UndeclaredShell(_))
    ));
}

#[test]
fn rejects_noncanonical_bootstrap_and_host_tool_closure() {
    let capsule = parse_capsule(CAPSULE).unwrap();
    let mut ctx = context();
    ctx.bootstrap_packages = &["other-bootstrap"];
    assert!(matches!(
        validate_capsules(&ctx, std::slice::from_ref(&capsule)),
        Err(ValidationError::NonCanonicalBootstrap(_))
    ));
    let mut ctx = context();
    ctx.product_closure_roles = &[PackageRole::HostTool];
    assert_eq!(
        validate_capsules(&ctx, &[capsule]),
        Err(ValidationError::HostToolInProductClosure)
    );
}
