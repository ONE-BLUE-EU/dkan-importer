use importer_lib::utils::{parse_latitude, parse_longitude};

#[test]
fn test_excel_validator_loose_dms_test() {
    let coord_1_latitude = parse_latitude("42.8708889").unwrap();
    let coord_1_longitude = parse_longitude("17.70386111").unwrap();
    // Check with tolerance for floating point precision
    assert!(
        (coord_1_latitude - 42.8708889).abs() < 1e-6,
        "Expected ~42.8708889, got {}",
        coord_1_latitude
    );
    assert!(
        (coord_1_longitude - 17.70386111).abs() < 1e-6,
        "Expected ~17.70386111, got {}",
        coord_1_longitude
    );

    let coord_2_latitude = parse_latitude("41 23.670 N").unwrap();
    let coord_2_longitude = parse_longitude("15.727E").unwrap();
    assert!(
        (coord_2_latitude - 41.3945).abs() < 1e-4,
        "Expected ~41.3945, got {}",
        coord_2_latitude
    );
    assert!(
        (coord_2_longitude - 15.727).abs() < 1e-6,
        "Expected ~15.727, got {}",
        coord_2_longitude
    );
}

#[test]
fn test_invalid_coordinate_parsing_fails() {
    // Test that obviously invalid coordinates fail to parse
    // Note: loose_dms library is very permissive, especially for longitude
    assert!(
        parse_latitude("invalid").is_err(),
        "invalid should not parse as latitude"
    );
}

#[test]
fn test_coordinate_parsing_with_extra_spaces() {
    // Test that coordinates with extra spaces before decimal point are handled correctly
    // This addresses the error: "41 23 .670 N" should be normalized to "41 23.670 N"
    let result = parse_latitude("41 23 .670 N");
    assert!(
        result.is_ok(),
        "Should parse latitude with space before decimal: {:?}",
        result
    );

    if let Ok(lat) = result {
        assert!(
            (lat - 41.3945).abs() < 1e-4,
            "Expected ~41.3945, got {}",
            lat
        );
    }

    // Test with multiple spaces
    let result2 = parse_latitude("41 23  .670 N");
    assert!(
        result2.is_ok(),
        "Should parse latitude with multiple spaces before decimal: {:?}",
        result2
    );

    // Test longitude with spaces
    let result3 = parse_longitude("15 .727 E");
    assert!(
        result3.is_ok(),
        "Should parse longitude with space before decimal: {:?}",
        result3
    );
}

#[test]
fn test_coordinate_parsing_with_multiple_spaces() {
    // Test that multiple consecutive spaces are collapsed
    let result = parse_latitude("41  23.670  N");
    assert!(
        result.is_ok(),
        "Should parse latitude with multiple spaces: {:?}",
        result
    );

    if let Ok(lat) = result {
        assert!(
            (lat - 41.3945).abs() < 1e-4,
            "Expected ~41.3945, got {}",
            lat
        );
    }
}
