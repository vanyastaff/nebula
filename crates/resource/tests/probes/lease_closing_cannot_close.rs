use nebula_resource::LeaseClosing;

// A lease observes its closing notice; it can never fire one.
fn fire(closing: &LeaseClosing) {
    closing.cancel();
}

fn main() {}
