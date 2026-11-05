//! Pinned database-lifetime bridge for native generation registration.

#[cfg(test)]
mod tests {
    use duckdb::Connection;

    #[test]
    fn native_registration_weak_handle_releases_database_and_isolates_connections() {
        crate::generation_probe_tests::abi::initialize();
        unsafe {
            let mut raw = std::ptr::null_mut();
            assert_eq!(
                libduckdb_sys::duckdb_open(std::ptr::null(), &mut raw),
                libduckdb_sys::DuckDBSuccess
            );
            let owner = Connection::open_from_raw(raw).unwrap();
            owner
                .execute_batch("CREATE TABLE retained AS SELECT 42 AS value")
                .unwrap();
            let database = std::sync::Arc::new(super::NativeDatabase::new(raw).unwrap());
            let value = database
                .with_connection(|connection| {
                    connection
                        .query_row("SELECT value FROM retained", [], |row| row.get::<_, i64>(0))
                        .map_err(|error| error.to_string())
                })
                .unwrap();
            assert_eq!(value, 42);
            let failed: Result<(), String> =
                database.with_connection(|_| Err("injected staging failure".into()));
            assert_eq!(failed.unwrap_err(), "injected staging failure");
            let independent = Connection::open_in_memory().unwrap();
            assert!(independent.prepare("SELECT * FROM retained").is_err());
            crate::live_tables::register(
                &owner,
                std::sync::Arc::new(spur_rest_table_gateway::vtab::bridge::IoBridge::new()),
                std::sync::Arc::clone(&database),
            )
            .unwrap();
            assert!(
                std::sync::Arc::strong_count(&database) > 1,
                "catalog retains callback state"
            );
            drop(owner);
            libduckdb_sys::duckdb_close(&mut raw);
            assert_eq!(
                std::sync::Arc::strong_count(&database),
                1,
                "database close releases callback state"
            );
            assert!(database
                .with_connection(|_| Ok(()))
                .unwrap_err()
                .contains("closed"));
            drop(database);
        }
    }
}

use std::{ffi::c_void, ptr::NonNull};

type Create = unsafe extern "C" fn(libduckdb_sys::duckdb_database) -> *mut c_void;
type Lock = unsafe extern "C" fn(*mut c_void) -> libduckdb_sys::duckdb_database;
type Destroy = unsafe extern "C" fn(*mut c_void);

#[derive(Clone, Copy)]
struct Api {
    create: Create,
    lock: Lock,
    destroy: Destroy,
    make_api: ResolveApi,
    destroy_api: Destroy,
}

type ResolveApi = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

impl Api {
    fn get() -> Self {
        #[cfg(not(test))]
        {
            extern "C" {
                fn spur_native_database_weak(
                    database: libduckdb_sys::duckdb_database,
                ) -> *mut c_void;
                fn spur_native_database_lock(handle: *mut c_void)
                    -> libduckdb_sys::duckdb_database;
                fn spur_native_database_weak_destroy(handle: *mut c_void);
                fn spur_native_api(anchor: *mut c_void) -> *mut c_void;
                fn spur_native_api_destroy(api: *mut c_void);
            }
            Self {
                create: spur_native_database_weak,
                lock: spur_native_database_lock,
                destroy: spur_native_database_weak_destroy,
                make_api: spur_native_api,
                destroy_api: spur_native_api_destroy,
            }
        }
        #[cfg(test)]
        unsafe {
            // The test loader contains the pinned engine and the same C++ bridge.
            // Resolve against that handle so tests do not link a second engine.
            Self {
                create: std::mem::transmute(crate::generation_probe_tests::abi::symbol(
                    c"spur_native_database_weak",
                )),
                lock: std::mem::transmute(crate::generation_probe_tests::abi::symbol(
                    c"spur_native_database_lock",
                )),
                destroy: std::mem::transmute(crate::generation_probe_tests::abi::symbol(
                    c"spur_native_database_weak_destroy",
                )),
                make_api: std::mem::transmute(crate::generation_probe_tests::abi::symbol(
                    c"spur_native_api",
                )),
                destroy_api: std::mem::transmute(crate::generation_probe_tests::abi::symbol(
                    c"spur_native_api_destroy",
                )),
            }
        }
    }
}

/// Initialize the pinned Rust binding by C symbol name, after identifying the
/// exact host through the stable prefix of its official extension API.
pub(crate) unsafe fn initialize_api(
    info: ffi::duckdb_extension_info,
    access: *const ffi::duckdb_extension_access,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = (*access).get_api.ok_or("missing extension API accessor")?(info, c"v1.2.0".as_ptr())
        as *const ffi::duckdb_ext_api_v1;
    if table.is_null() {
        return Err("host extension API unavailable".into());
    }
    // The library-version field belongs to the unchanged stable v1.2.0 prefix.
    let version_fn = (*table)
        .duckdb_library_version
        .ok_or("missing host version")?;
    let version = std::ffi::CStr::from_ptr(version_fn());
    if version != c"v1.4.4" && version != c"v1.5.5" {
        return Err(
            "native source registration requires a qualified DuckDB v1.4.4 or v1.5.5 runtime"
                .into(),
        );
    }
    let api = Api::get();
    let resolved = (api.make_api)(version_fn as *mut c_void);
    if resolved.is_null() {
        return Err("qualified host does not export the pinned DuckDB API".into());
    }
    struct Resolved(*mut c_void, Destroy);
    impl Drop for Resolved {
        fn drop(&mut self) {
            unsafe { (self.1)(self.0) }
        }
    }
    let resolved = Resolved(resolved, api.destroy_api);
    unsafe extern "C" fn get_api(
        table: ffi::duckdb_extension_info,
        _: *const std::ffi::c_char,
    ) -> *const c_void {
        table.cast()
    }
    let shim = ffi::duckdb_extension_access {
        set_error: None,
        get_database: None,
        get_api: Some(get_api),
    };
    if !ffi::duckdb_rs_extension_api_init(resolved.0.cast(), &shim, "v1.2.0")? {
        return Err("cannot initialize resolved host API".into());
    }
    Ok(())
}

pub(crate) struct NativeDatabase {
    handle: NonNull<c_void>,
    api: Api,
}

// SAFETY: the C++ object contains only an immutable weak_ptr. lock() is const,
// thread-safe, and creates an independent owning database wrapper per call.
unsafe impl Send for NativeDatabase {}
unsafe impl Sync for NativeDatabase {}

impl NativeDatabase {
    /// The caller must supply a live database from the exact pinned engine.
    pub(crate) unsafe fn new(database: libduckdb_sys::duckdb_database) -> Result<Self, String> {
        let api = Api::get();
        let handle = NonNull::new((api.create)(database))
            .ok_or("cannot create native database weak handle")?;
        Ok(Self { handle, api })
    }

    fn with_database<T>(
        &self,
        run: impl FnOnce(libduckdb_sys::duckdb_database) -> Result<T, String>,
    ) -> Result<T, String> {
        struct Lease(libduckdb_sys::duckdb_database);
        impl Drop for Lease {
            fn drop(&mut self) {
                // SAFETY: this wrapper is owned by this lease and outlives its connection.
                unsafe { libduckdb_sys::duckdb_close(&mut self.0) }
            }
        }
        // SAFETY: handle is valid until self drops; C++ returns a distinct owned wrapper.
        let raw = unsafe { (self.api.lock)(self.handle.as_ptr()) };
        if raw.is_null() {
            return Err("native database is closed or cannot be leased".into());
        }
        let lease = Lease(raw);
        run(lease.0)
    }

    #[cfg(test)]
    fn with_connection<T>(
        &self,
        run: impl FnOnce(&duckdb::Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_database(|raw| {
            // SAFETY: this connection drops before the enclosing database lease.
            let connection = unsafe { duckdb::Connection::open_from_raw(raw) }
                .map_err(|error| error.to_string())?;
            run(&connection)
        })
    }

    pub(crate) fn register(&self, name: &str, extra: ApiTableExtra) -> Result<(), String> {
        let name = std::ffi::CString::new(name).map_err(|error| error.to_string())?;
        self.with_database(|database| unsafe {
            struct Connection(ffi::duckdb_connection);
            impl Drop for Connection {
                fn drop(&mut self) {
                    unsafe { ffi::duckdb_disconnect(&mut self.0) }
                }
            }
            let mut connection = Connection(std::ptr::null_mut());
            if ffi::duckdb_connect(database, &mut connection.0) != ffi::DuckDBSuccess {
                return Err("cannot connect for native generation registration".into());
            }
            struct Function(ffi::duckdb_table_function);
            impl Drop for Function {
                fn drop(&mut self) {
                    unsafe { ffi::duckdb_destroy_table_function(&mut self.0) }
                }
            }
            let function = Function(ffi::duckdb_create_table_function());
            ffi::duckdb_table_function_set_name(function.0, name.as_ptr());
            for parameter in &extra.filter_parameters {
                let name = std::ffi::CString::new(parameter.name.as_str())
                    .map_err(|error| error.to_string())?;
                let kind = match parameter.data_type {
                    arrow_schema::DataType::Int64 => ffi::DUCKDB_TYPE_DUCKDB_TYPE_BIGINT,
                    arrow_schema::DataType::Float64 => ffi::DUCKDB_TYPE_DUCKDB_TYPE_DOUBLE,
                    arrow_schema::DataType::Boolean => ffi::DUCKDB_TYPE_DUCKDB_TYPE_BOOLEAN,
                    arrow_schema::DataType::Utf8 => ffi::DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR,
                    _ => return Err("unsupported native argument type".into()),
                };
                let mut ty = ffi::duckdb_create_logical_type(kind);
                ffi::duckdb_table_function_add_named_parameter(function.0, name.as_ptr(), ty);
                ffi::duckdb_destroy_logical_type(&mut ty);
            }
            ffi::duckdb_table_function_set_extra_info(
                function.0,
                Box::into_raw(Box::new(extra)).cast(),
                Some(drop_box::<ApiTableExtra>),
            );
            ffi::duckdb_table_function_set_bind(function.0, Some(bind));
            ffi::duckdb_table_function_set_init(function.0, Some(init));
            ffi::duckdb_table_function_set_function(function.0, Some(scan));
            if ffi::duckdb_register_table_function(connection.0, function.0) != ffi::DuckDBSuccess {
                return Err("cannot register immutable native generation function".into());
            }
            Ok(())
        })
    }
}

impl Drop for NativeDatabase {
    fn drop(&mut self) {
        // SAFETY: exactly one owner releases the weak handle.
        unsafe { (self.api.destroy)(self.handle.as_ptr()) }
    }
}

use duckdb::{
    core::DataChunkHandle,
    vtab::{BindInfo, InitInfo, TableFunctionInfo, VTab},
};
use libduckdb_sys as ffi;
use spur_rest_table_gateway::{
    adapter::{Predicate, PredicateOp, ScalarValue},
    vtab::table_fn::{
        arrow_to_duckdb_type, scan_api_table, ApiBindData, ApiInitData, ApiTableExtra, ApiTableVTab,
    },
};

unsafe extern "C" fn drop_box<T>(value: *mut c_void) {
    drop(Box::from_raw(value.cast::<T>()));
}

// DuckDB owns each returned value. SQL NULL is omission at this boundary; the
// string "NULL" is a real string, never a sentinel. The pinned duckdb-rs Value
// wrapper does not expose null inspection, so use the supported C API here.
unsafe fn bind_data(raw: ffi::duckdb_bind_info) -> Result<ApiBindData, Box<dyn std::error::Error>> {
    let bind = BindInfo::from(raw);
    let extra = &*bind.get_extra_info::<ApiTableExtra>();
    let mut predicates = Vec::new();
    for parameter in &extra.filter_parameters {
        let name = std::ffi::CString::new(parameter.name.as_str())?;
        struct Value(ffi::duckdb_value);
        impl Drop for Value {
            fn drop(&mut self) {
                unsafe { ffi::duckdb_destroy_value(&mut self.0) }
            }
        }
        let value = Value(ffi::duckdb_bind_get_named_parameter(raw, name.as_ptr()));
        if value.0.is_null() || ffi::duckdb_is_null_value(value.0) {
            if parameter.required {
                return Err(format!("missing required native argument {}", parameter.name).into());
            }
            continue;
        }
        let value = match parameter.data_type {
            arrow_schema::DataType::Int64 => ScalarValue::Int64(ffi::duckdb_get_int64(value.0)),
            arrow_schema::DataType::Float64 => {
                ScalarValue::Float64(ffi::duckdb_get_double(value.0))
            }
            arrow_schema::DataType::Boolean => ScalarValue::Bool(ffi::duckdb_get_bool(value.0)),
            arrow_schema::DataType::Utf8 => {
                let text = ffi::duckdb_get_varchar(value.0);
                if text.is_null() {
                    return Err("cannot read native string argument".into());
                }
                let result = std::ffi::CStr::from_ptr(text).to_str().map(str::to_owned);
                ffi::duckdb_free(text.cast());
                ScalarValue::Utf8(result?)
            }
            _ => return Err("unsupported native argument type".into()),
        };
        predicates.push(Predicate {
            column: parameter.name.clone(),
            op: PredicateOp::Eq,
            value,
        });
    }
    for field in extra.schema.fields() {
        bind.add_result_column(
            field.name(),
            arrow_to_duckdb_type(field.data_type())?.into(),
        );
    }
    Ok(ApiBindData::from_extra(extra, predicates))
}

fn callback_error(
    result: std::thread::Result<Result<(), Box<dyn std::error::Error>>>,
) -> Option<std::ffi::CString> {
    let message = match result {
        Ok(Ok(())) => return None,
        Ok(Err(error)) => error.to_string(),
        Err(_) => "native table callback panicked".into(),
    };
    Some(std::ffi::CString::new(message.replace('\0', "\\0")).expect("NUL escaped"))
}

unsafe extern "C" fn bind(raw: ffi::duckdb_bind_info) {
    let result = std::panic::catch_unwind(|| {
        let data = bind_data(raw)?;
        BindInfo::from(raw).set_bind_data(
            Box::into_raw(Box::new(data)).cast(),
            Some(drop_box::<ApiBindData>),
        );
        Ok(())
    });
    if let Some(error) = callback_error(result) {
        ffi::duckdb_bind_set_error(raw, error.as_ptr());
    }
}

unsafe extern "C" fn init(raw: ffi::duckdb_init_info) {
    let result = std::panic::catch_unwind(|| {
        let info = InitInfo::from(raw);
        let data = ApiTableVTab::init(&info)?;
        info.set_init_data(
            Box::into_raw(Box::new(data)).cast(),
            Some(drop_box::<ApiInitData>),
        );
        Ok(())
    });
    if let Some(error) = callback_error(result) {
        ffi::duckdb_init_set_error(raw, error.as_ptr());
    }
}

unsafe extern "C" fn scan(raw: ffi::duckdb_function_info, output: ffi::duckdb_data_chunk) {
    let result = std::panic::catch_unwind(|| {
        let info = TableFunctionInfo::<ApiTableVTab>::from(raw);
        let types = info
            .get_bind_data()
            .schema
            .fields()
            .iter()
            .map(|field| arrow_to_duckdb_type(field.data_type()).map(Into::into))
            .collect::<Result<Vec<_>, _>>()?;
        // DataChunkHandle has no public borrowed constructor. Reference its
        // vectors through C API (shared buffers, no copying or private layout).
        let mut chunk = DataChunkHandle::new(&types);
        scan_api_table(info.get_bind_data(), info.get_init_data(), &mut chunk)?;
        for column in 0..types.len() {
            ffi::duckdb_vector_reference_vector(
                ffi::duckdb_data_chunk_get_vector(output, column as u64),
                ffi::duckdb_data_chunk_get_vector(chunk.get_ptr(), column as u64),
            );
        }
        ffi::duckdb_data_chunk_set_size(output, chunk.len() as u64);
        Ok(())
    });
    if let Some(error) = callback_error(result) {
        ffi::duckdb_function_set_error(raw, error.as_ptr());
    }
}
