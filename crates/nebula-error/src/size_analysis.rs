//! Size analysis and debugging for error types

use std::mem::size_of;

pub fn analyze_sizes() {
    println!("\n=== V1 NebulaError Size Analysis ===");
    println!("Total size: {} bytes", size_of::<crate::NebulaError>());
    println!(
        "  Box<ErrorKind>: {} bytes",
        size_of::<Box<crate::ErrorKind>>()
    );
    println!(
        "  Option<Box<ErrorContext>>: {} bytes",
        size_of::<Option<Box<crate::ErrorContext>>>()
    );
    println!(
        "  Cow<'static, str>: {} bytes",
        size_of::<std::borrow::Cow<'static, str>>()
    );
    println!(
        "  Option<Duration>: {} bytes",
        size_of::<Option<std::time::Duration>>()
    );
    println!("  bool: {} bytes", size_of::<bool>());

    println!("\n=== V2 NebulaErrorV2 Size Analysis ===");
    println!(
        "Total size: {} bytes",
        size_of::<crate::optimized::NebulaErrorV2>()
    );
    println!(
        "  ErrorKindV2: {} bytes",
        size_of::<crate::optimized::ErrorKindV2>()
    );
    println!("  SmolStr (code): {} bytes", size_of::<smol_str::SmolStr>());
    println!(
        "  SmolStr (message): {} bytes",
        size_of::<smol_str::SmolStr>()
    );
    println!(
        "  ErrorFlags: {} bytes",
        size_of::<crate::optimized::ErrorFlags>()
    );
    println!("  u16 (retry_delay_ms): {} bytes", size_of::<u16>());
    println!(
        "  Option<Box<ErrorContextV2>>: {} bytes",
        size_of::<Option<Box<crate::optimized::ErrorContextV2>>>()
    );

    println!("\n=== ErrorKind Variants Size ===");
    println!("V1 ErrorKind enum: {} bytes", size_of::<crate::ErrorKind>());
    println!(
        "  Box<ErrorKind>: {} bytes",
        size_of::<Box<crate::ErrorKind>>()
    );
    println!(
        "V2 ErrorKindV2 enum: {} bytes",
        size_of::<crate::optimized::ErrorKindV2>()
    );

    println!("\n=== Individual V2 Error Variants ===");
    println!(
        "ClientErrorV2: {} bytes",
        size_of::<crate::optimized::ClientErrorV2>()
    );
    println!(
        "ServerErrorV2: {} bytes",
        size_of::<crate::optimized::ServerErrorV2>()
    );
    println!(
        "InfraErrorV2: {} bytes",
        size_of::<crate::optimized::InfraErrorV2>()
    );
    println!(
        "DomainErrorV2: {} bytes",
        size_of::<crate::optimized::DomainErrorV2>()
    );

    println!("\n=== Context Size Analysis ===");
    println!(
        "V1 ErrorContext: {} bytes",
        size_of::<crate::ErrorContext>()
    );
    println!(
        "V2 ErrorContextV2: {} bytes",
        size_of::<crate::optimized::ErrorContextV2>()
    );
    println!(
        "  SmolStr (description): {} bytes",
        size_of::<smol_str::SmolStr>()
    );
    println!(
        "  SmallVec metadata: {} bytes",
        size_of::<smallvec::SmallVec<[(smol_str::SmolStr, smol_str::SmolStr); 4]>>()
    );
    println!(
        "  ContextIds: {} bytes",
        size_of::<crate::optimized::ContextIds>()
    );
    println!("  Option<u64>: {} bytes", size_of::<Option<u64>>());

    println!("\n=== String Type Comparison ===");
    println!("String: {} bytes", size_of::<String>());
    println!(
        "Cow<'static, str>: {} bytes",
        size_of::<std::borrow::Cow<'static, str>>()
    );
    println!("SmolStr: {} bytes", size_of::<smol_str::SmolStr>());
    println!("Box<str>: {} bytes", size_of::<Box<str>>());

    println!("\n=== Diagnosis ===");
    let v1_size = size_of::<crate::NebulaError>();
    let v2_size = size_of::<crate::optimized::NebulaErrorV2>();

    if v2_size > v1_size {
        println!("❌ V2 is {} bytes LARGER than V1!", v2_size - v1_size);
        println!("   Problem: ErrorKindV2 likely contains large SmolStr fields");
        println!("   Solution: Box the ErrorKindV2 like V1 does");
    } else {
        println!("✅ V2 is {} bytes smaller than V1", v1_size - v2_size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_size_analysis() {
        analyze_sizes();
    }
}
