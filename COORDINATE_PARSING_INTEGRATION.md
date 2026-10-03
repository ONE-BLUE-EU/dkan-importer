# Coordinate Parsing Integration

## Overview

The coordinate parsing functionality from `importer-lib/src/utils/coordinate.rs` has been successfully integrated into the Excel row processing pipeline. When processing Excel files, latitude and longitude values are now automatically parsed through the `parse_latitude()` and `parse_longitude()` functions, which return f64 values. The coordinate values are used as-is without any formatting or rounding applied.

## What Was Implemented

### 1. String Coordinate Processing

When Excel cells contain string values in latitude or longitude fields, they are automatically processed through the coordinate parser:

- **Decimal format**: `"42.8708889"` → `42.8708889` (f64, used as-is)
- **DMS format**: `"41 23.670 N"` → `41.3945` (f64, used as-is)
- **DMS with extra spaces**: `"41 23 .670 N"` → `41.3945` (f64, normalized and parsed)
- **Degrees with direction**: `"15.727E"` → `15.727` (f64, used as-is)

**Space Normalization:**
- Multiple consecutive spaces are collapsed to single spaces
- Spaces before decimal points are removed (e.g., `"23 .670"` → `"23.670"`)
- Commas are converted to decimal points for localization support

Coordinate values are returned as f64 numbers without any formatting or precision adjustment.

### 2. Numeric Coordinate Processing

When Excel cells contain numeric values in latitude or longitude fields, they are also processed through the coordinate parser. The numeric value is converted to a string, parsed through the coordinate parser (which handles normalization and validation), and the resulting f64 value is used.

### 3. Field Name Recognition

The system automatically recognizes coordinate fields based on their names (case-insensitive):

**Latitude fields:**
- `"Latitude"`, `"latitude"`, `"LATITUDE"`
- `"lat"`, `"Lat"`, `"LAT"`
- `"Latitude*"` (with asterisk for required fields)

**Longitude fields:**
- `"Longitude"`, `"longitude"`, `"LONGITUDE"`
- `"lng"`, `"Lng"`, `"LNG"`
- `"lon"`, `"Lon"`, `"LON"`
- `"Longitude*"` (with asterisk for required fields)

### 4. Schema-Aware Type Conversion

The coordinate parser respects the JSON schema:

- If the field is defined as `"type": "number"`, the parsed coordinate (f64) is returned as a numeric value
- If the field is defined as `"type": "string"`, the parsed coordinate is returned as a numeric value
- Mixed types are also supported
- Values are used as-is without formatting or rounding

## Code Changes

### Modified Files

1. **`importer-lib/src/excel_validator.rs`**
   - Modified `convert_string_with_schema_intelligence()` to detect and parse latitude/longitude strings
   - Modified `convert_numeric_with_validation()` to detect and parse latitude/longitude numeric values
   - Made these methods `pub(crate)` to enable unit testing
   - Added 6 comprehensive unit tests for coordinate parsing

## Test Coverage

Added comprehensive test coverage with 10 new tests:

1. **`test_numeric_latitude_longitude_parsing`**
   - Tests numeric coordinate values are used as-is

2. **`test_string_latitude_longitude_parsing_dms_format`**
   - Tests DMS format coordinate strings (e.g., "41 23.670 N")

3. **`test_string_latitude_longitude_parsing_decimal_format`**
   - Tests decimal format coordinate strings

4. **`test_string_field_coordinate_parsing`**
   - Tests coordinates in string-type schema fields

5. **`test_coordinate_field_name_variations`**
   - Tests various field name patterns (lat, lng, lon)

6. **`test_coordinate_parsing_with_asterisk_fields`**
   - Tests coordinate parsing with required fields (asterisk suffix)

7. **`test_invalid_latitude_returns_original_value`**
   - Tests graceful handling of unparseable latitude values

8. **`test_valid_coordinates_use_values_as_is`**
   - Tests that coordinates maintain full precision

9. **`test_coordinate_parsing_with_extra_spaces`**
   - Tests handling of coordinates with spaces before decimal point (e.g., "41 23 .670 N")

10. **`test_coordinate_parsing_with_multiple_spaces`**
    - Tests handling of multiple consecutive spaces in coordinates

All existing tests continue to pass. Total: **107 tests** (72 unit + 35 integration).

## Usage Example

When processing an Excel file with the following schema:

```json
{
  "type": "object",
  "properties": {
    "Sample ID": {
      "type": "string"
    },
    "Latitude": {
      "type": "number",
      "minimum": -90.0,
      "maximum": 90.0
    },
    "Longitude": {
      "type": "number",
      "minimum": -180.0,
      "maximum": 180.0
    }
  },
  "required": ["Sample ID"]
}
```

Excel data like this:

| Sample ID | Latitude     | Longitude    |
|-----------|--------------|--------------|
| S001      | 42.8708889   | 17.70386111  |
| S002      | 41 23.670 N  | 15.727E      |

Will be automatically parsed to:

```json
[
  {
    "Sample ID": "S001",
    "Latitude": 42.870889,
    "Longitude": 17.703861
  },
  {
    "Sample ID": "S002",
    "Latitude": 41.394500,
    "Longitude": 15.727000
  }
]
```

## Implementation Details

### Coordinate Detection Logic

The system checks if a field name (converted to lowercase) contains:
- For latitude: `"latitude"` or `"lat"`
- For longitude: `"longitude"`, `"lng"`, or `"lon"`

### Parsing Flow

1. **For String Values:**
   - Detect if field name indicates a coordinate
   - Call `parse_latitude()` or `parse_longitude()` from utils (returns `Result<f64, Error>`)
   - If parsing succeeds, return the f64 value as-is
   - If parsing fails, log a warning and return the original string value (validation will catch type mismatches)

2. **For Numeric Values:**
   - Detect if field name indicates a coordinate
   - Convert number to string
   - Call `parse_latitude()` or `parse_longitude()` from utils (returns `Result<f64, Error>`)
   - If parsing succeeds, return the f64 value as-is (no rounding or formatting applied)
   - If parsing fails, log a warning and return the original value

### Error Handling

- If coordinate parsing fails, the original value is returned unchanged
- Enhanced warning messages are logged when parsing fails, including:
  - The invalid value
  - The field name
  - The parsing error details
  - Helpful format suggestions (e.g., "should be in decimal degrees (e.g., '41.3945') or DMS format (e.g., '41 23.670 N')")
- The system gracefully handles invalid coordinate formats
- Validation errors are reported through the existing validation framework
- **No panics**: Parsing failures do not stop execution; they result in validation errors if the value doesn't match the schema

## Benefits

1. **Automatic Parsing**: All coordinates are automatically parsed and normalized
2. **Format Flexibility**: Supports multiple coordinate input formats (decimal, DMS, etc.)
3. **Zero Configuration**: No changes needed to existing code using the importer
4. **Backward Compatible**: Non-coordinate fields are unaffected
5. **Type Safe**: Respects JSON schema type definitions
6. **Full Precision**: Maintains full floating-point precision without any formatting or rounding
7. **Graceful Failures**: Invalid coordinates are reported as validation errors, not runtime panics

## Testing

Run all tests:
```bash
cd importer-lib
cargo test
```

Run only coordinate tests:
```bash
cargo test --lib test_coordinate
cargo test --lib test_numeric_latitude_longitude_parsing
cargo test --lib test_string_latitude_longitude_parsing
cargo test --lib test_string_field_coordinate_parsing
```

## Dependencies

The coordinate parsing relies on:
- `loose_dms` crate (already included in dependencies)
- `importer-lib/src/utils/coordinate.rs` module (already implemented)

## Notes

- The `convert_string_with_schema_intelligence` and `convert_numeric_with_validation` methods are now `pub(crate)` to allow unit testing from within the crate
- The `parse_latitude()` and `parse_longitude()` functions return `Result<f64, Box<dyn std::error::Error>>`
- Coordinate values are used as-is without any formatting or rounding applied
- Full floating-point precision is maintained for all coordinate fields
- For coordinate fields, the `multipleOf` rounding constraint is not applied
- Parsing failures result in warnings being logged and the original value being returned
- Invalid coordinate values will be caught during JSON schema validation if the schema requires a number
- The integration is transparent to external API consumers - no API changes required

