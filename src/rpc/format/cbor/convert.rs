use ciborium::Value as Data;
use geo::{LineString, Point, Polygon};
use geo_types::{MultiLineString, MultiPoint, MultiPolygon};
use std::collections::BTreeMap;
use std::iter::once;
use std::ops::Deref;
use surrealdb::sql::Datetime;
use surrealdb::sql::Duration;
use surrealdb::sql::Geometry;
use surrealdb::sql::Id;
use surrealdb::sql::Number;
use surrealdb::sql::Thing;
use surrealdb::sql::Uuid;
use surrealdb::sql::Value;

const TAG_DATETIME: u64 = 0;
const TAG_NONE: u64 = 6;
const TAG_UUID: u64 = 7;
const TAG_DECIMAL: u64 = 8;
const TAG_DURATION: u64 = 9;
const TAG_RECORDID: u64 = 10;
const TAG_TABLE: u64 = 11;
const TAG_GEOMETRY_POINT: u64 = 88;
const TAG_GEOMETRY_LINE: u64 = 89;
const TAG_GEOMETRY_POLYGON: u64 = 90;
const TAG_GEOMETRY_MULTIPOINT: u64 = 91;
const TAG_GEOMETRY_MULTILINE: u64 = 92;
const TAG_GEOMETRY_MULTIPOLYGON: u64 = 93;
const TAG_GEOMETRY_COLLECTION: u64 = 94;

#[derive(Debug)]
pub struct Cbor(pub Data);

impl TryFrom<Cbor> for Value {
	type Error = &'static str;
	fn try_from(val: Cbor) -> Result<Self, &'static str> {
		match val.0 {
			Data::Null => Ok(Value::Null),
			Data::Bool(v) => Ok(Value::from(v)),
			Data::Integer(v) => Ok(Value::from(i128::from(v))),
			Data::Float(v) => Ok(Value::from(v)),
			Data::Bytes(v) => Ok(Value::Bytes(v.into())),
			Data::Text(v) => Ok(Value::from(v)),
			Data::Array(v) => {
				v.into_iter().map(|v| Value::try_from(Cbor(v))).collect::<Result<Value, &str>>()
			}
			Data::Map(v) => v
				.into_iter()
				.map(|(k, v)| {
					let k = Value::try_from(Cbor(k)).map(|k| k.as_raw_string());
					let v = Value::try_from(Cbor(v));
					Ok((k?, v?))
				})
				.collect::<Result<Value, &str>>(),
			Data::Tag(t, v) => {
				match t {
					// A literal datetime
					TAG_DATETIME => match *v {
						Data::Text(v) => match Datetime::try_from(v) {
							Ok(v) => Ok(v.into()),
							_ => Err("Expected a valid Datetime value"),
						},
						_ => Err("Expected a CBOR text data type"),
					},
					// A literal NONE
					TAG_NONE => Ok(Value::None),
					// A literal uuid
					TAG_UUID => match *v {
						Data::Text(v) => match Uuid::try_from(v) {
							Ok(v) => Ok(v.into()),
							_ => Err("Expected a valid UUID value"),
						},
						_ => Err("Expected a CBOR text data type"),
					},
					// A literal decimal
					TAG_DECIMAL => match *v {
						Data::Text(v) => match Number::try_from(v) {
							Ok(v) => Ok(v.into()),
							_ => Err("Expected a valid Decimal value"),
						},
						_ => Err("Expected a CBOR text data type"),
					},
					// A literal duration
					TAG_DURATION => match *v {
						Data::Text(v) => match Duration::try_from(v) {
							Ok(v) => Ok(v.into()),
							_ => Err("Expected a valid Duration value"),
						},
						_ => Err("Expected a CBOR text data type"),
					},
					// A literal recordid
					TAG_RECORDID => match *v {
						Data::Text(v) => match Thing::try_from(v) {
							Ok(v) => Ok(v.into()),
							_ => Err("Expected a valid RecordID value"),
						},
						Data::Array(mut v) if v.len() == 2 => match (v.remove(0), v.remove(0)) {
							(Data::Text(tb), Data::Text(id)) => {
								Ok(Value::from(Thing::from((tb, id))))
							}
							(Data::Text(tb), Data::Integer(id)) => {
								Ok(Value::from(Thing::from((tb, Id::from(i128::from(id) as i64)))))
							}
							(Data::Text(tb), Data::Array(id)) => Ok(Value::from(Thing::from((
								tb,
								Id::from(
									id.into_iter()
										.map(|v| Value::try_from(Cbor(v)))
										.collect::<Result<Vec<Value>, &str>>()?,
								),
							)))),
							(Data::Text(tb), Data::Map(id)) => Ok(Value::from(Thing::from((
								tb,
								Id::from(
									id.into_iter()
										.map(|(k, v)| {
											let k =
												Value::try_from(Cbor(k)).map(|k| k.as_raw_string());
											let v = Value::try_from(Cbor(v));
											Ok((k?, v?))
										})
										.collect::<Result<BTreeMap<String, Value>, &str>>()?,
								),
							)))),
							_ => Err("Expected a CBOR array with 2 elements, a text data type, and a valid ID type"),
						},
						_ => Err("Expected a CBOR text data type, or a CBOR array with 2 elements"),
					},
					// A literal table
					TAG_TABLE => match *v {
						Data::Text(v) => Ok(Value::Table(v.into())),
						_ => Err("Expected a CBOR text data type"),
					},
					TAG_GEOMETRY_POINT => match v.deref() {
						Data::Array(v) if v.len() == 2 => {
							let x = Value::try_from(Cbor(v.clone().remove(0)))?;
							let y = Value::try_from(Cbor(v.clone().remove(0)))?;

							match (x, y) {
								(Value::Number(x), Value::Number(y)) => {
									Ok(Value::Geometry(Geometry::Point((x.as_float(), y.as_float()).into())))
								},
								_ => Err("Expected a CBOR array with 2 decimal values"),
							}
						},
						_ => Err("Expected a CBOR array with 2 decimal values"),
					},
					TAG_GEOMETRY_LINE => match v.deref() {
						Data::Array(v) => {
							let points = v
								.iter()
								.map(|v| match Value::try_from(Cbor(v.clone()))? {
									Value::Geometry(Geometry::Point(v)) => Ok(v),
									_ => Err("Expected a CBOR array with Geometry Point values")
								})
								.collect::<Result<Vec<Point>, &str>>()?;

							Ok(Value::Geometry(Geometry::Line(LineString::from(points))))
						},
						_ => Err("Expected a CBOR array with Geometry Point values"),
					},
					TAG_GEOMETRY_POLYGON => match v.deref() {
						Data::Array(v) if v.len() >= 2 => {
							let lines = v
								.iter()
								.map(|v| match Value::try_from(Cbor(v.clone()))? {
									Value::Geometry(Geometry::Line(v)) => Ok(v),
									_ => Err("Expected a CBOR array with Geometry Line values")
								})
								.collect::<Result<Vec<LineString>, &str>>()?;

							let first = match lines.first() {
								Some(v) => v,
								_ => return Err("Expected a CBOR array with at least two Geometry Line values")
							};

							Ok(Value::Geometry(Geometry::Polygon(Polygon::new(first.clone(), Vec::from(&lines[1..])))))
						},
						_ => Err("Expected a CBOR array with at least two Geometry Line values"),
					},
					TAG_GEOMETRY_MULTIPOINT => match v.deref() {
						Data::Array(v) => {
							let points = v
								.iter()
								.map(|v| match Value::try_from(Cbor(v.clone()))? {
									Value::Geometry(Geometry::Point(v)) => Ok(v),
									_ => Err("Expected a CBOR array with Geometry Point values")
								})
								.collect::<Result<Vec<Point>, &str>>()?;

							Ok(Value::Geometry(Geometry::MultiPoint(MultiPoint::from(points))))
						},
						_ => Err("Expected a CBOR array with Geometry Point values"),
					},
					TAG_GEOMETRY_MULTILINE => match v.deref() {
						Data::Array(v) => {
							let lines = v
								.iter()
								.map(|v| match Value::try_from(Cbor(v.clone()))? {
									Value::Geometry(Geometry::Line(v)) => Ok(v),
									_ => Err("Expected a CBOR array with Geometry Line values")
								})
								.collect::<Result<Vec<LineString>, &str>>()?;

							Ok(Value::Geometry(Geometry::MultiLine(MultiLineString::new(lines))))
						},
						_ => Err("Expected a CBOR array with Geometry Point values"),
					},
					TAG_GEOMETRY_MULTIPOLYGON => match v.deref() {
						Data::Array(v) => {
							let polygons = v
								.iter()
								.map(|v| match Value::try_from(Cbor(v.clone()))? {
									Value::Geometry(Geometry::Polygon(v)) => Ok(v),
									_ => Err("Expected a CBOR array with Geometry Polygon values")
								})
								.collect::<Result<Vec<Polygon>, &str>>()?;

							Ok(Value::Geometry(Geometry::MultiPolygon(MultiPolygon::from(polygons))))
						},
						_ => Err("Expected a CBOR array with Geometry Polygon values"),
					},
					TAG_GEOMETRY_COLLECTION => match v.deref() {
						Data::Array(v) => {
							let geometries = v
								.iter()
								.map(|v| match Value::try_from(Cbor(v.clone()))? {
									Value::Geometry(v) => Ok(v),
									_ => Err("Expected a CBOR array with Geometry values")
								})
								.collect::<Result<Vec<Geometry>, &str>>()?;

							Ok(Value::Geometry(Geometry::Collection(geometries)))
						},
						_ => Err("Expected a CBOR array with Geometry values"),
					},
					// An unknown tag
					_ => Err("Encountered an unknown CBOR tag"),
				}
			}
			_ => Err("Encountered an unknown CBOR data type"),
		}
	}
}

impl TryFrom<Value> for Cbor {
	type Error = &'static str;
	fn try_from(val: Value) -> Result<Self, &'static str> {
		match val {
			Value::None => Ok(Cbor(Data::Tag(TAG_NONE, Box::new(Data::Null)))),
			Value::Null => Ok(Cbor(Data::Null)),
			Value::Bool(v) => Ok(Cbor(Data::Bool(v))),
			Value::Number(v) => match v {
				Number::Int(v) => Ok(Cbor(Data::Integer(v.into()))),
				Number::Float(v) => Ok(Cbor(Data::Float(v))),
				Number::Decimal(v) => {
					Ok(Cbor(Data::Tag(TAG_DECIMAL, Box::new(Data::Text(v.to_string())))))
				}
			},
			Value::Strand(v) => Ok(Cbor(Data::Text(v.0))),
			Value::Duration(v) => {
				Ok(Cbor(Data::Tag(TAG_DURATION, Box::new(Data::Text(v.to_raw())))))
			}
			Value::Datetime(v) => {
				Ok(Cbor(Data::Tag(TAG_DATETIME, Box::new(Data::Text(v.to_raw())))))
			}
			Value::Uuid(v) => Ok(Cbor(Data::Tag(TAG_UUID, Box::new(Data::Text(v.to_raw()))))),
			Value::Array(v) => Ok(Cbor(Data::Array(
				v.into_iter()
					.map(|v| {
						let v = Cbor::try_from(v)?.0;
						Ok(v)
					})
					.collect::<Result<Vec<Data>, &str>>()?,
			))),
			Value::Object(v) => Ok(Cbor(Data::Map(
				v.into_iter()
					.map(|(k, v)| {
						let k = Data::Text(k);
						let v = Cbor::try_from(v)?.0;
						Ok((k, v))
					})
					.collect::<Result<Vec<(Data, Data)>, &str>>()?,
			))),
			Value::Bytes(v) => Ok(Cbor(Data::Bytes(v.into_inner()))),
			Value::Thing(v) => Ok(Cbor(Data::Tag(
				TAG_RECORDID,
				Box::new(Data::Array(vec![
					Data::Text(v.tb),
					match v.id {
						Id::Number(v) => Data::Integer(v.into()),
						Id::String(v) => Data::Text(v),
						Id::Array(v) => Cbor::try_from(Value::from(v))?.0,
						Id::Object(v) => Cbor::try_from(Value::from(v))?.0,
						Id::Generate(_) => {
							return Err("Cannot encode an ungenerated Record ID into CBOR")
						}
					},
				])),
			))),
			Value::Table(v) => Ok(Cbor(Data::Tag(TAG_TABLE, Box::new(Data::Text(v.0))))),
			Value::Geometry(v) => Ok(Cbor(encode_geometry(v))),
			// We shouldn't reach here
			_ => Err("Found unsupported SurrealQL value being encoded into a CBOR value"),
		}
	}
}

fn encode_geometry(v: Geometry) -> Data {
	match v {
		Geometry::Point(v) => Data::Tag(
			TAG_GEOMETRY_POINT,
			Box::new(Data::Array(vec![
				Data::Tag(TAG_DECIMAL, Box::new(Data::Text(v.x().to_string()))),
				Data::Tag(TAG_DECIMAL, Box::new(Data::Text(v.y().to_string()))),
			])),
		),
		Geometry::Line(v) => {
			let data = v.points().map(|v| encode_geometry(v.into())).collect::<Vec<Data>>();

			Data::Tag(TAG_GEOMETRY_LINE, Box::new(Data::Array(data)))
		}
		Geometry::Polygon(v) => {
			let data = once(v.exterior())
				.chain(v.interiors())
				.map(|v| encode_geometry(v.clone().into()))
				.collect::<Vec<Data>>();

			Data::Tag(TAG_GEOMETRY_POLYGON, Box::new(Data::Array(data)))
		}
		Geometry::MultiPoint(v) => {
			let data = v.iter().map(|v| encode_geometry((*v).into())).collect::<Vec<Data>>();

			Data::Tag(TAG_GEOMETRY_MULTIPOINT, Box::new(Data::Array(data)))
		}
		Geometry::MultiLine(v) => {
			let data = v.iter().map(|v| encode_geometry(v.clone().into())).collect::<Vec<Data>>();

			Data::Tag(TAG_GEOMETRY_MULTILINE, Box::new(Data::Array(data)))
		}
		Geometry::MultiPolygon(v) => {
			let data = v.iter().map(|v| encode_geometry(v.clone().into())).collect::<Vec<Data>>();

			Data::Tag(TAG_GEOMETRY_MULTIPOLYGON, Box::new(Data::Array(data)))
		}
		Geometry::Collection(v) => {
			let data = v.iter().map(|v| encode_geometry(v.clone())).collect::<Vec<Data>>();

			Data::Tag(TAG_GEOMETRY_COLLECTION, Box::new(Data::Array(data)))
		}
	}
}
