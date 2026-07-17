use std::fmt;
use std::marker::PhantomData;

/// Represents the database field type.
///
/// Analogous to the various field classes in Django (e.g., `CharField`, `IntegerField`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Text field with a maximum length. Django's `CharField`.
    Char,
    /// Unbounded text field. Django's `TextField`.
    Text,
    /// Standard 32-bit signed integer. Django's `IntegerField`.
    Integer,
    /// 64-bit signed integer. Django's `BigIntegerField`.
    BigInt,
    /// Floating point number. Django's `FloatField`.
    Float,
    /// Fixed-precision decimal number. Django's `DecimalField`.
    Decimal {
        /// The maximum number of digits allowed in the number.
        max_digits: u32,
        /// The number of decimal places to store with the number.
        decimal_places: u32,
    },
    /// Boolean field. Django's `BooleanField`.
    Boolean,
    /// Date field. Django's `DateField`.
    Date,
    /// Date and time field. Django's `DateTimeField`.
    DateTime,
    /// Time field. Django's `TimeField`.
    Time,
    /// Duration / delta time field. Django's `DurationField`.
    Duration,
    /// Universally unique identifier. Django's `UUIDField`.
    Uuid,
    /// Email address field. Django's `EmailField`.
    Email,
    /// URL field. Django's `URLField`.
    Url,
    /// Slug field (for URLs). Django's `SlugField`.
    Slug,
    /// IP address field. Django's `GenericIPAddressField`.
    Ip,
    /// Binary data field. Django's `BinaryField`.
    Binary,
    /// JSON-encoded data field. Django's `JSONField`.
    Json,
}

/// Represents the default value of a model field.
///
/// Analogous to the `default` argument in Django model fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefaultValue {
    /// Integer default value.
    I64(i64),
    /// Floating point default value.
    F64(f64),
    /// Text default value.
    Text(&'static str),
    /// Boolean default value.
    Bool(bool),
    /// No default value.
    None,
}

/// Metadata for a single model field.
///
/// Contains information about the field's name, type, database column, nullability,
/// constraints, and defaults. Analogous to attributes on a Django Field instance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldMeta {
    /// The name of the field on the Rust struct.
    pub name: &'static str,
    /// The database column name for this field.
    pub column_name: &'static str,
    /// The database field type.
    pub kind: FieldKind,
    /// Whether the field is allowed to be NULL. Django's `null` parameter.
    pub nullable: bool,
    /// Whether this field is the primary key. Django's `primary_key` parameter.
    pub primary_key: bool,
    /// Whether the field is auto-incremented or auto-generated. Django's `AutoField`.
    pub auto: bool,
    /// Whether the field values must be unique across the table. Django's `unique` parameter.
    pub unique: bool,
    /// Whether a database index should be created for this field. Django's `db_index` parameter.
    pub db_index: bool,
    /// The default value for the field. Django's `default` parameter.
    pub default: DefaultValue,
    /// The maximum length in characters (useful for Char fields). Django's `max_length` parameter.
    pub max_length: Option<u32>,
    /// A human-readable name for the field. Django's `verbose_name` parameter.
    pub verbose_name: Option<&'static str>,
    /// Extra helper text for forms. Django's `help_text` parameter.
    pub help_text: Option<&'static str>,
    /// Choices for field values, as (db_value, human_readable_label) pairs. Django's `choices` parameter.
    pub choices: &'static [(&'static str, &'static str)],
}

/// The kind of relation between models.
///
/// Matches Django's relational field types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    /// Many-to-one relation. Django's `ForeignKey`.
    ForeignKey,
    /// One-to-one relation. Django's `OneToOneField`.
    OneToOne,
    /// Many-to-many relation. Django's `ManyToManyField`.
    ManyToMany,
}

/// Action to take when a referenced object is deleted.
///
/// Analogous to Django's `on_delete` behaviors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDelete {
    /// Cascade deletes. Django's `CASCADE`.
    Cascade,
    /// Prevent deletion of referenced object. Django's `PROTECT`.
    Protect,
    /// Set the reference to NULL. Django's `SET_NULL`.
    SetNull,
    /// Prevent deletion of referenced object (SQL standard). Django's `RESTRICT`.
    Restrict,
    /// Take no action. Django's `DO_NOTHING`.
    DoNothing,
}

/// Metadata for a relation between two models.
///
/// Analogous to Django's relational fields.
#[derive(Debug, Clone, Copy)]
pub struct RelationMeta {
    /// The name of the relation field on the Rust struct.
    pub field_name: &'static str,
    /// The relationship kind (e.g. ForeignKey).
    pub kind: RelationKind,
    /// Late-bound target model metadata function. Resolves ordering circularity.
    pub target: fn() -> &'static ModelMeta,
    /// The referential integrity delete rule. Django's `on_delete`.
    pub on_delete: OnDelete,
    /// The name to use for the relation from the related object back to this one. Django's `related_name`.
    pub related_name: Option<&'static str>,
}

/// Metadata for an explicit database index.
///
/// Analogous to entries in Django's `Meta.indexes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexMeta {
    /// The name of the index in the database.
    pub name: &'static str,
    /// The fields included in this index.
    pub fields: &'static [&'static str],
}

/// Runtime metadata for a model.
///
/// This serves as the single source of truth for downstream crates (migrations, admin, forms, etc.).
/// Analogous to Django's `Model._meta` options class.
#[derive(Debug, Clone, Copy)]
pub struct ModelMeta {
    /// The name of the Rust struct representing this model.
    pub struct_name: &'static str,
    /// The app label defining the logical namespace. Django's `app_label`.
    pub app_label: &'static str,
    /// The database table name. Django's `db_table`.
    pub table_name: &'static str,
    /// Fields defined on this model.
    pub fields: &'static [FieldMeta],
    /// Relations defined on this model.
    pub relations: &'static [RelationMeta],
    /// Indexes defined on this model.
    pub indexes: &'static [IndexMeta],
    /// Unique constraints across multiple fields. Django's `unique_together`.
    pub unique_together: &'static [&'static [&'static str]],
    /// The default ordering for querysets. Django's `ordering`.
    pub ordering: &'static [&'static str],
}

/// Trait implemented by all models to expose their metadata.
pub trait Model {
    /// Returns the static `ModelMeta` reference for this model.
    fn meta() -> &'static ModelMeta
    where
        Self: Sized;
}

/// Entry representing a registered model.
///
/// Used by the `inventory` crate to collect all models at startup.
pub struct ModelRegistration {
    /// Function returning the model's metadata.
    pub meta_fn: fn() -> &'static ModelMeta,
}

inventory::collect!(ModelRegistration);

/// Returns an iterator over all registered model metadata.
///
/// Can be used by downstream tools like migrations to discover all models.
pub fn all_registered_models() -> impl Iterator<Item = &'static ModelMeta> {
    inventory::iter::<ModelRegistration>().map(|r| (r.meta_fn)())
}

/// A wrapper representing a foreign key relationship to another model.
///
/// Analogous to Django's `ForeignKey` field.
pub struct ForeignKey<T: Model> {
    /// The ID of the related model instance.
    pub id: i64,
    /// Marker to associate this key with the target model type.
    _marker: PhantomData<T>,
}

impl<T: Model> ForeignKey<T> {
    /// Creates a new `ForeignKey` reference.
    pub fn new(id: i64) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }
}

impl<T: Model> Clone for ForeignKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Model> Copy for ForeignKey<T> {}

impl<T: Model> fmt::Debug for ForeignKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForeignKey").field("id", &self.id).finish()
    }
}
