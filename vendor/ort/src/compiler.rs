use core::{
	ffi::c_void,
	marker::PhantomData,
	mem,
	ops::Deref,
	ptr::{self, NonNull}
};
#[cfg(feature = "std")]
use std::path::Path;

use crate::{
	AsPointer,
	environment::get_environment,
	error::{Error, Result},
	memory::Allocator,
	ortsys,
	session::builder::SessionBuilder,
	util::OnceLock
};

/// Returns a pointer to the global [`ort_sys::OrtCompileApi`] object, or errors if the Compile API is not
/// supported.
pub fn compile_api() -> Result<&'static ort_sys::OrtCompileApi> {
	struct CompileApiPointer(*const ort_sys::OrtCompileApi);
	unsafe impl Send for CompileApiPointer {}
	unsafe impl Sync for CompileApiPointer {}

	static COMPILE_API: OnceLock<CompileApiPointer> = OnceLock::new();

	let ptr = NonNull::new(
		COMPILE_API
			.get_or_init(|| {
				let api = ortsys![unsafe GetCompileApi()];
				CompileApiPointer(api)
			})
			.0
			.cast_mut()
	)
	.ok_or_else(|| Error::new("The Compile API is not supported with this backend."))?;
	Ok(unsafe { ptr.as_ref() })
}

pub struct ModelCompiler<'i> {
	ptr: NonNull<ort_sys::OrtModelCompilationOptions>,
	_session_options: SessionBuilder,
	#[cfg(feature = "managed-environment")]
	_environment: crate::environment::EnvironmentLease,
	_p: PhantomData<&'i ()>
}

// CompileModel may publish the output before a later initialization failure.
#[cfg(feature = "managed-environment")]
struct PendingCompiledBuffer<'a> {
	ptr: *mut c_void,
	allocator: &'a Allocator
}

#[cfg(feature = "managed-environment")]
impl Drop for PendingCompiledBuffer<'_> {
	fn drop(&mut self) {
		if !self.ptr.is_null() {
			unsafe { self.allocator.free(self.ptr) };
			#[cfg(feature = "owner-test-hooks")]
			crate::owner_test_hooks::record_compiler_buffer_free();
		}
	}
}

impl<'i> ModelCompiler<'i> {
	pub fn new(options: SessionBuilder) -> Result<Self> {
		let env = get_environment()?;
		let mut ptr = ptr::null_mut();
		ortsys![@compile:
			unsafe CreateModelCompilationOptionsFromSessionOptions(
				env.ptr(),
				options.ptr(),
				&mut ptr
			)?;
			nonNull(ptr)
		];
		Ok(Self {
			ptr,
			_session_options: options,
			#[cfg(feature = "managed-environment")]
			_environment: env,
			_p: PhantomData
		})
	}

	#[cfg(feature = "std")]
	#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
	pub fn with_model_from_file<P: AsRef<Path>>(self, path: P) -> Result<Self> {
		let model_path = crate::util::path_to_os_char(path);
		ortsys![@compile:
			unsafe ModelCompilationOptions_SetInputModelPath(
				self.ptr.as_ptr(),
				model_path.as_ptr()
			)?
		];
		Ok(self)
	}

	pub fn with_model_from_memory<'i2>(self, data: &'i2 [u8]) -> Result<ModelCompiler<'i2>> {
		ortsys![@compile:
			unsafe ModelCompilationOptions_SetInputModelFromBuffer(
				self.ptr.as_ptr(),
				data.as_ptr().cast(),
				data.len()
			)?
		];
		Ok(unsafe { mem::transmute::<ModelCompiler<'i>, ModelCompiler<'i2>>(self) })
	}

	pub fn with_embed_ep_context(self, enable: bool) -> Result<Self> {
		ortsys![@compile:
			unsafe ModelCompilationOptions_SetEpContextEmbedMode(
				self.ptr.as_ptr(),
				enable
			)?
		];
		Ok(self)
	}

	#[cfg(feature = "std")]
	#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
	pub fn with_external_initializers<P: AsRef<Path>>(self, threshold: usize, path: P) -> Result<Self> {
		let model_path = crate::util::path_to_os_char(path);
		ortsys![@compile:
			unsafe ModelCompilationOptions_SetOutputModelExternalInitializersFile(
				self.ptr.as_ptr(),
				model_path.as_ptr(),
				threshold
			)?
		];
		Ok(self)
	}

	#[cfg(feature = "std")]
	#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
	pub fn compile_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
		let env = get_environment()?;
		let model_path = crate::util::path_to_os_char(path);
		ortsys![@compile:
			unsafe ModelCompilationOptions_SetOutputModelPath(
				self.ptr.as_ptr(),
				model_path.as_ptr()
			)?
		];
		ortsys![@compile: unsafe CompileModel(env.ptr(), self.ptr.as_ptr())?];
		Ok(())
	}

	pub fn compile_to_buffer(&self) -> Result<CompiledModel> {
		let env = get_environment()?;
		#[cfg(feature = "managed-environment")]
		let allocator = Allocator::default();
		#[cfg(not(feature = "managed-environment"))]
		let mut allocator = Allocator::default();
		#[cfg(feature = "managed-environment")]
		let allocator_ptr = allocator.ptr().cast_mut();
		#[cfg(not(feature = "managed-environment"))]
		let allocator_ptr = allocator.ptr_mut();
		#[cfg(not(feature = "managed-environment"))]
		let mut ptr = ptr::null_mut();
		#[cfg(feature = "managed-environment")]
		let mut pending = PendingCompiledBuffer { ptr: ptr::null_mut(), allocator: &allocator };
		#[cfg(feature = "managed-environment")]
		let output_ptr = &mut pending.ptr;
		#[cfg(not(feature = "managed-environment"))]
		let output_ptr = &mut ptr;
		let mut size = 0;
		ortsys![@compile:
			unsafe ModelCompilationOptions_SetOutputModelBuffer(
				self.ptr.as_ptr(),
				allocator_ptr,
				output_ptr,
				&mut size
			)?
		];
		ortsys![@compile: unsafe CompileModel(env.ptr(), self.ptr.as_ptr())?];
		#[cfg(feature = "owner-test-hooks")]
		if !output_ptr.is_null() {
			crate::owner_test_hooks::fail_if_requested(4)?;
		}
		#[cfg(feature = "managed-environment")]
		let ptr = core::mem::replace(&mut pending.ptr, ptr::null_mut());
		#[cfg(feature = "managed-environment")]
		drop(pending);
		Ok(CompiledModel { ptr, size, allocator })
	}
}

impl AsPointer for ModelCompiler<'_> {
	type Sys = ort_sys::OrtModelCompilationOptions;
	fn ptr(&self) -> *const Self::Sys {
		self.ptr.as_ptr()
	}
}

impl Drop for ModelCompiler<'_> {
	fn drop(&mut self) {
		ortsys![@compile: unsafe ReleaseModelCompilationOptions(self.ptr.as_ptr())];
	}
}

pub struct CompiledModel {
	ptr: *mut c_void,
	size: usize,
	allocator: Allocator
}

impl CompiledModel {
	pub fn as_slice(&self) -> &[u8] {
		unsafe { core::slice::from_raw_parts(self.ptr.cast(), self.size) }
	}
}

impl Deref for CompiledModel {
	type Target = [u8];
	fn deref(&self) -> &Self::Target {
		self.as_slice()
	}
}

impl Drop for CompiledModel {
	fn drop(&mut self) {
		unsafe { self.allocator.free(self.ptr) };
	}
}
