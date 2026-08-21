use sim_platform_core::RequirementBuilder;

#[derive(Debug, Eq, PartialEq)]
struct DescriptorBoot<'a> {
    argv: &'a [&'a str],
    site: &'a str,
    bound_services: &'a [&'a str],
}

fn boot<'a>(argv: &'a [&'a str], descriptor: &'a str) -> DescriptorBoot<'a> {
    match descriptor {
        "model" => DescriptorBoot {
            argv,
            site: "platform/site/model",
            bound_services: &["platform/monotonic-clock", "platform/entropy"],
        },
        "ubuntu" => DescriptorBoot {
            argv,
            site: "platform/site/ubuntu-pc-desktop",
            bound_services: &[
                "platform/monotonic-clock",
                "platform/entropy",
                "platform/open",
            ],
        },
        _ => panic!("unknown checked descriptor"),
    }
}

#[test]
fn same_pure_command_binds_descriptor_specific_services() {
    let args = &["sim", "platform", "require", "platform/monotonic-clock"];
    let requirement = RequirementBuilder::new().require(args[3]).unwrap().build();
    assert_eq!(requirement.len(), 1);
    let model = boot(args, "model");
    let ubuntu = boot(args, "ubuntu");
    assert_eq!(model.argv, ubuntu.argv);
    assert_ne!(model.site, ubuntu.site);
    assert_ne!(model.bound_services, ubuntu.bound_services);
}
