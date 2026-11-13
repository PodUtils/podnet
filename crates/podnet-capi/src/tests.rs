use super::*;

fn last() -> String {
    unsafe { CStr::from_ptr(podnet_last_error()) }
        .to_str()
        .unwrap()
        .to_string()
}

#[test]
fn a_panic_is_mapped_to_the_panic_code_and_the_error_text() {
    let code = guard(|| -> Res<c_int> { panic!("planted {}", 7) });
    assert_eq!(code, E_PANIC);
    assert!(last().contains("planted 7"), "{}", last());
    let code = guard(|| -> Res<c_int> { std::panic::panic_any(5u8) });
    assert_eq!(code, E_PANIC);
    assert_eq!(guard(|| Ok(3)), 3);
    assert_eq!(last(), "");
}

#[test]
fn the_error_text_is_cut_to_64_kib_on_a_character_boundary() {
    let text = "é".repeat(MAX_ERROR);
    set_error(&text);
    let got = last();
    assert!(got.len() < MAX_ERROR, "{}", got.len());
    assert!(got.chars().all(|c| c == 'é'));
    set_error("a\0b");
    assert_eq!(last(), "a?b");
}

#[test]
fn a_null_argument_is_a_bad_argument() {
    unsafe {
        assert_eq!(
            podnet_link_new(std::ptr::null(), std::ptr::null(), std::ptr::null_mut()),
            E_ARG
        );
        assert!(last().contains("out is NULL"), "{}", last());
        assert_eq!(
            podnet_stream_read(std::ptr::null_mut(), std::ptr::null_mut(), 1),
            E_ARG
        );
        assert_eq!(podnet_link_close(std::ptr::null_mut()), E_ARG);
        assert!(podnet_stream_service(std::ptr::null()).is_null());
    }
}

#[test]
fn init_is_idempotent_and_a_bad_config_is_refused() {
    unsafe {
        let mut err: *const c_char = std::ptr::null();
        assert_eq!(podnet_init(c"{\"threads\": 0}".as_ptr(), &mut err), E_ARG);
        assert!(!err.is_null());
        assert_eq!(
            podnet_init(c"{\"x\": 1}".as_ptr(), std::ptr::null_mut()),
            E_ARG
        );
        assert_eq!(podnet_init(c"{".as_ptr(), std::ptr::null_mut()), E_ARG);
        assert_eq!(podnet_init(std::ptr::null(), std::ptr::null_mut()), OK);
        assert_eq!(
            podnet_init(c"{\"threads\": 3}".as_ptr(), std::ptr::null_mut()),
            OK
        );
        assert_eq!(podnet_init(c"".as_ptr(), std::ptr::null_mut()), OK);
    }
}

#[test]
fn the_version_is_the_version_of_the_crate() {
    let v = unsafe { CStr::from_ptr(podnet_version()) }
        .to_str()
        .unwrap();
    assert_eq!(v, env!("CARGO_PKG_VERSION"));
}
