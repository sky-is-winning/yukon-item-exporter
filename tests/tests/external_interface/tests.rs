use crate::external_interface::ExternalInterfaceTestProvider;
use crate::set_logger;
use crate::util::options::TestOptions;
use crate::util::test::Test;
use ruffle_core::external::Value as ExternalValue;
use std::collections::BTreeMap;
use std::path::Path;

pub fn external_interface_avm1() -> Result<(), libtest_mimic::Failed> {
    set_logger();
    Test::from_options(
        TestOptions {
            num_frames: Some(1),
            ..Default::default()
        },
        Path::new("tests/swfs/avm1/external_interface/"),
        "external_interface_avm1".to_string(),
    )?
    .run(
        |player| {
            player
                .lock()
                .unwrap()
                .add_external_interface(Box::new(ExternalInterfaceTestProvider::new()));
            Ok(())
        },
        |player| {
            let mut player_locked = player.lock().unwrap();

            let parroted =
                player_locked.call_internal_interface("parrot", vec!["Hello World!".into()]);
            player_locked.log_backend().avm_trace(&format!(
                "After calling `parrot` with a string: {parroted:?}",
            ));

            let mut nested = BTreeMap::new();
            nested.insert(
                "list".to_string(),
                vec![
                    "string".into(),
                    100.into(),
                    false.into(),
                    ExternalValue::Object(BTreeMap::new()),
                ]
                .into(),
            );

            let mut root = BTreeMap::new();
            root.insert("number".to_string(), (-500.1).into());
            root.insert("string".to_string(), "A string!".into());
            root.insert("true".to_string(), true.into());
            root.insert("false".to_string(), false.into());
            root.insert("null".to_string(), ExternalValue::Null);
            root.insert("nested".to_string(), nested.into());
            let result = player_locked
                .call_internal_interface("callWith", vec!["trace".into(), root.into()]);
            player_locked.log_backend().avm_trace(&format!(
                "After calling `callWith` with a complex payload: {result:?}",
            ));
            Ok(())
        },
    )
}

pub fn external_interface_avm2() -> Result<(), libtest_mimic::Failed> {
    set_logger();
    Test::from_options(
        TestOptions {
            num_frames: Some(1),
            ..Default::default()
        },
        Path::new("tests/swfs/avm2/external_interface/"),
        "external_interface_avm2".to_string(),
    )?
    .run(
        |player| {
            player
                .lock()
                .unwrap()
                .add_external_interface(Box::new(ExternalInterfaceTestProvider::new()));
            Ok(())
        },
        |player| {
            let mut player_locked = player.lock().unwrap();

            let parroted =
                player_locked.call_internal_interface("parrot", vec!["Hello World!".into()]);
            player_locked.log_backend().avm_trace(&format!(
                "After calling `parrot` with a string: {parroted:?}",
            ));

            player_locked.call_internal_interface("freestanding", vec!["Hello World!".into()]);

            let root: ExternalValue = vec![
                "string".into(),
                100.into(),
                ExternalValue::Null,
                false.into(),
            ]
            .into();

            let result =
                player_locked.call_internal_interface("callWith", vec!["trace".into(), root]);
            player_locked.log_backend().avm_trace(&format!(
                "After calling `callWith` with a complex payload: {result:?}",
            ));
            Ok(())
        },
    )
}
