use diesel::AsExpression;
use diesel::deserialize::{self, FromSql, FromSqlRow};
use diesel::serialize::{self, IsNull, Output, ToSql};
use diesel::sql_types::{BigInt, Text};
use diesel::sqlite::{Sqlite, SqliteValue};
use uuid::Uuid;

#[derive(Debug, Clone, FromSqlRow, AsExpression)]
#[diesel(sql_type = Text)]
pub struct UuidSql(pub Uuid);

impl From<Uuid> for UuidSql {
    fn from(value: Uuid) -> Self {
        UuidSql(value)
    }
}

impl TryFrom<UuidSql> for Uuid {
    type Error = uuid::Error;

    fn try_from(value: UuidSql) -> Result<Self, Self::Error> {
        Ok(value.0)
    }
}

impl FromSql<Text, Sqlite> for UuidSql {
    fn from_sql(value: SqliteValue<'_, '_, '_>) -> deserialize::Result<Self> {
        let raw = <String as FromSql<Text, Sqlite>>::from_sql(value)?;
        Ok(UuidSql(Uuid::parse_str(&raw)?))
    }
}

impl ToSql<Text, Sqlite> for UuidSql {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        out.set_value(self.0.to_string());
        Ok(IsNull::No)
    }
}

#[derive(Debug, Clone, FromSqlRow, AsExpression)]
#[diesel(sql_type = BigInt)]
pub struct U64Sql(pub u64);

impl From<u64> for U64Sql {
    fn from(value: u64) -> Self {
        U64Sql(value)
    }
}

impl TryFrom<U64Sql> for u64 {
    type Error = std::num::TryFromIntError;

    fn try_from(value: U64Sql) -> Result<Self, Self::Error> {
        Ok(value.0)
    }
}

impl FromSql<BigInt, Sqlite> for U64Sql {
    fn from_sql(mut value: SqliteValue<'_, '_, '_>) -> deserialize::Result<Self> {
        Ok(U64Sql(u64::try_from(value.read_long())?))
    }
}

impl ToSql<BigInt, Sqlite> for U64Sql {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        out.set_value(i64::try_from(self.0)?);
        Ok(IsNull::No)
    }
}
