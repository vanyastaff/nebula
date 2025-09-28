//! Универсальный макрос для создания валидаторов с любым количеством параметров

#[macro_export]
macro_rules! validator {
    // ==================== БЕЗ ПАРАМЕТРОВ ====================
    (
        $(#[$attr:meta])*
        $vis:vis struct $name:ident $(<$($generic:ident),*>)? {
        }
        impl {
            fn check($value:ident: &Value) -> bool {
                $check_body:block
            }
            fn error() -> String {
                $error_body:block
            }
            const DESCRIPTION: &str = $desc:expr;
        }
    ) => {
        $(#[$attr])*
        #[derive(Debug, Clone)]
        $vis struct $name $(<$($generic),*>)? {
            name: String,
        }

        impl $(<$($generic),*>)? $name $(<$($generic),*>)? {
            pub fn new() -> Self {
                Self {
                    name: stringify!($name).to_lowercase(),
                }
            }

            pub fn with_name(mut self, name: impl Into<String>) -> Self {
                self.name = name.into();
                self
            }

            fn check($value: &nebula_value::Value) -> bool {
                $check_body
            }

            fn error() -> String {
                $error_body
            }
        }

        #[async_trait::async_trait]
        impl $(<$($generic),*>)? $crate::core::Validator for $name $(<$($generic),*>)?
        {
            async fn validate(
                &self,
                value: &nebula_value::Value,
                _context: Option<&$crate::core::ValidationContext>,
            ) -> Result<$crate::core::Valid<()>, $crate::core::Invalid<()>> {
                if Self::check(value) {
                    Ok($crate::core::Valid::simple(()))
                } else {
                    let msg = Self::error();
                    Err($crate::core::Invalid::simple(msg))
                }
            }

            fn name(&self) -> &str {
                &self.name
            }

            fn description(&self) -> Option<&str> {
                Some($desc)
            }
        }
    };

    // ==================== С ПАРАМЕТРАМИ ====================
    (
        $(#[$attr:meta])*
        $vis:vis struct $name:ident $(<$($generic:ident),*>)? {
            $($field:ident: $field_ty:ty),+
        }
        impl {
            fn check($value:ident: &Value, $($param:ident: &$param_ty:ty),+) -> bool {
                $check_body:block
            }
            fn error($($param_err:ident: &$param_err_ty:ty),+) -> String {
                $error_body:block
            }
            const DESCRIPTION: &str = $desc:expr;
        }
    ) => {
        $(#[$attr])*
        #[derive(Debug, Clone)]
        $vis struct $name $(<$($generic),*>)? {
            $(pub $field: $field_ty),+,
            name: String,
        }

        impl $(<$($generic),*>)? $name $(<$($generic),*>)? {
            pub fn new($($field: $field_ty),+) -> Self {
                Self {
                    $($field: $field.clone()),+,
                    name: format!("{}_{}",
                        stringify!($name).to_lowercase(),
                        vec![$(format!("{:?}", $field)),+].join("_")
                    ),
                }
            }

            pub fn with_name(mut self, name: impl Into<String>) -> Self {
                self.name = name.into();
                self
            }

            $(pub fn $field(&self) -> &$field_ty {
                &self.$field
            })+

            fn check($value: &nebula_value::Value, $($param: &$param_ty),+) -> bool {
                $check_body
            }

            fn error($($param_err: &$param_err_ty),+) -> String {
                $error_body
            }
        }

        #[async_trait::async_trait]
        impl $(<$($generic),*>)? $crate::core::Validator for $name $(<$($generic),*>)?
        where
            $($field_ty: Clone + Send + Sync),+
        {
            async fn validate(
                &self,
                value: &nebula_value::Value,
                _context: Option<&$crate::core::ValidationContext>,
            ) -> Result<$crate::core::Valid<()>, $crate::core::Invalid<()>> {
                if Self::check(value, $(&self.$field),+) {
                    Ok($crate::core::Valid::simple(()))
                } else {
                    let msg = Self::error($(&self.$field),+);
                    Err($crate::core::Invalid::simple(msg))
                }
            }

            fn name(&self) -> &str {
                &self.name
            }

            fn description(&self) -> Option<&str> {
                Some($desc)
            }
        }
    };

    // ==================== С КОНТЕКСТОМ ====================
    (
        $(#[$attr:meta])*
        $vis:vis struct $name:ident $(<$($generic:ident),*>)? {
            $($field:ident: $field_ty:ty),+
        }
        impl {
            fn check($value:ident: &Value, $($param:ident: &$param_ty:ty),+, $ctx:ident: Option<&ValidationContext>) -> bool {
                $check_body:block
            }
            fn error($($param_err_ctx:ident: &$param_err_ctx_ty:ty),+) -> String {
                $error_body:block
            }
            const DESCRIPTION: &str = $desc:expr;
        }
    ) => {
        $(#[$attr])*
        #[derive(Debug, Clone)]
        $vis struct $name $(<$($generic),*>)? {
            $(pub $field: $field_ty),+,
            name: String,
        }

        impl $(<$($generic),*>)? $name $(<$($generic),*>)? {
            pub fn new($($field: $field_ty),+) -> Self {
                Self {
                    $($field: $field.clone()),+,
                    name: format!("{}_{}",
                        stringify!($name).to_lowercase(),
                        vec![$(format!("{:?}", $field)),+].join("_")
                    ),
                }
            }

            pub fn with_name(mut self, name: impl Into<String>) -> Self {
                self.name = name.into();
                self
            }

            $(pub fn $field(&self) -> &$field_ty {
                &self.$field
            })+

            fn check($value: &nebula_value::Value, $($param: &$param_ty),+, $ctx: Option<&$crate::core::ValidationContext>) -> bool {
                $check_body
            }

            fn error($($param_err_ctx: &$param_err_ctx_ty),+) -> String {
                $error_body
            }
        }

        #[async_trait::async_trait]
        impl $(<$($generic),*>)? $crate::core::Validator for $name $(<$($generic),*>)?
        where
            $($field_ty: Clone + Send + Sync),+
        {
            async fn validate(
                &self,
                value: &nebula_value::Value,
                context: Option<&$crate::core::ValidationContext>,
            ) -> Result<$crate::core::Valid<()>, $crate::core::Invalid<()>> {
                if Self::check(value, $(&self.$field),+, context) {
                    Ok($crate::core::Valid::simple(()))
                } else {
                    let msg = Self::error($(&self.$field),+);
                    Err($crate::core::Invalid::simple(msg))
                }
            }

            fn name(&self) -> &str {
                &self.name
            }

            fn description(&self) -> Option<&str> {
                Some($desc)
            }
        }
    };

    // ==================== С ОБЯЗАТЕЛЬНЫМ КОНТЕКСТОМ ====================
    (
        $(#[$attr:meta])*
        $vis:vis struct $name:ident $(<$($generic:ident),*>)? {
            $($field:ident: $field_ty:ty),+
        }
        impl {
            fn check_with_context($value:ident: &Value, $ctx:ident: &ValidationContext, $($param:ident: &$param_ty:ty),+) -> bool {
                $check_body:block
            }
            fn error($($param_err_ctx:ident: &$param_err_ctx_ty:ty),+) -> String {
                $error_body:block
            }
            const DESCRIPTION: &str = $desc:expr;
        }
    ) => {
        $(#[$attr])*
        #[derive(Debug, Clone)]
        $vis struct $name $(<$($generic),*>)? {
            $(pub $field: $field_ty),+,
            name: String,
        }

        impl $(<$($generic),*>)? $name $(<$($generic),*>)? {
            pub fn new($($field: $field_ty),+) -> Self {
                Self {
                    $($field: $field.clone()),+,
                    name: format!("{}_{}",
                        stringify!($name).to_lowercase(),
                        vec![$(format!("{:?}", $field)),+].join("_")
                    ),
                }
            }

            pub fn with_name(mut self, name: impl Into<String>) -> Self {
                self.name = name.into();
                self
            }

            $(pub fn $field(&self) -> &$field_ty {
                &self.$field
            })+

            fn check_with_context($value: &nebula_value::Value, $ctx: &$crate::core::ValidationContext, $($param: &$param_ty),+) -> bool {
                $check_body
            }

            fn error($($param_err_ctx: &$param_err_ctx_ty),+) -> String {
                $error_body
            }
        }

        #[async_trait::async_trait]
        impl $(<$($generic),*>)? $crate::core::Validator for $name $(<$($generic),*>)?
        where
            $($field_ty: Clone + Send + Sync),+
        {
            async fn validate(
                &self,
                value: &nebula_value::Value,
                context: Option<&$crate::core::ValidationContext>,
            ) -> Result<$crate::core::Valid<()>, $crate::core::Invalid<()>> {
                if let Some(ctx) = context {
                    if Self::check_with_context(value, ctx, $(&self.$field),+) {
                        Ok($crate::core::Valid::simple(()))
                    } else {
                        let msg = Self::error($(&self.$field),+);
                        Err($crate::core::Invalid::simple(msg))
                    }
                } else {
                    Err($crate::core::Invalid::simple(
                        format!("{} validator requires validation context", stringify!($name))
                    ))
                }
            }

            fn name(&self) -> &str {
                &self.name
            }

            fn description(&self) -> Option<&str> {
                Some($desc)
            }
        }
    };
}

// ==================== ВСПОМОГАТЕЛЬНЫЕ МАКРОСЫ ====================

/// Макрос для создания convenience функций
#[macro_export]
macro_rules! validator_fn {
    // Без параметров
    ($vis:vis fn $fn_name:ident() -> $validator:ty) => {
        $vis fn $fn_name() -> $validator {
            <$validator>::new()
        }
    };

    // С параметрами
    ($vis:vis fn $fn_name:ident($($param:ident: $param_ty:ty),*) -> $validator:ty) => {
        $vis fn $fn_name($($param: $param_ty),*) -> $validator {
            <$validator>::new($($param),*)
        }
    };
}

/// Макрос для быстрого создания модуля с валидаторами
#[macro_export]
macro_rules! validators_module {
    (
        $(#[$mod_attr:meta])*
        $vis:vis mod $mod_name:ident {
            $($validator:item)*
        }
    ) => {
        $(#[$mod_attr])*
        $vis mod $mod_name {
            use super::*;
            $($validator)*
        }
    };
}