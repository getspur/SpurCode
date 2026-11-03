use ort::operator::kernel::KernelAttributes;

#[cfg(feature = "managed-environment")]
#[test]
fn managed_attributes_do_not_implement_clone() {
	trait AmbiguousIfClone<A> { fn marker() {} }
	impl<T: ?Sized> AmbiguousIfClone<()> for T {}
	impl<T: ?Sized + Clone> AmbiguousIfClone<u8> for T {}
	let _ = <KernelAttributes as AmbiguousIfClone<_>>::marker;
}

#[cfg(not(feature = "managed-environment"))]
#[test]
fn unmanaged_attributes_implement_clone() {
	fn assert_clone<T: Clone>() {}
	assert_clone::<KernelAttributes>();
}
