// nebula_widgets/src/framework/ticker_mixin.rs

use nebula_animation::*;
use nebula_core::*;
use std::sync::{Arc, Mutex, Weak};

//=============================================================================
// SingleTickerProviderStateMixin - like Flutter
//=============================================================================

/// Mixin for State that provides a single Ticker
/// Usage: impl SingleTickerProviderStateMixin for MyState
pub trait SingleTickerProviderStateMixin: State {
    /// Get ticker (created automatically)
    fn ticker(&self) -> &Option<Ticker>;
    
    /// Get mutable ticker
    fn ticker_mut(&mut self) -> &mut Option<Ticker>;
    
    /// Check if widget is mounted
    fn is_mounted(&self) -> bool;
}

/// Helper to implement SingleTickerProviderStateMixin
#[derive(Default)]
pub struct SingleTickerProviderState {
    ticker: Option<Ticker>,
    mounted: bool,
}

impl SingleTickerProviderState {
    pub fn new() -> Self {
        Self {
            ticker: None,
            mounted: false,
        }
    }
    
    pub fn create_ticker<F>(&mut self, on_tick: F) -> Ticker 
    where
        F: FnMut(Duration) -> TickerFuture + Send + 'static,
    {
        assert!(self.ticker.is_none(), "Cannot create multiple tickers");
        
        let ticker = Ticker::new(Box::new(on_tick));
        self.ticker = Some(ticker.clone());
        ticker
    }
    
    pub fn dispose_ticker(&mut self) {
        if let Some(ticker) = &mut self.ticker {
            ticker.dispose();
        }
        self.ticker = None;
    }
}

//=============================================================================
// TickerProviderStateMixin - for multiple tickers
//=============================================================================

/// Mixin for State that provides multiple Tickers
pub trait TickerProviderStateMixin: State {
    /// Get ticker manager
    fn ticker_manager(&self) -> &TickerManager;
    
    /// Get mutable ticker manager
    fn ticker_manager_mut(&mut self) -> &mut TickerManager;
}

/// Manager for multiple tickers
pub struct TickerManager {
    tickers: Vec<Ticker>,
    mounted: bool,
}

impl TickerManager {
    pub fn new() -> Self {
        Self {
            tickers: Vec::new(),
            mounted: false,
        }
    }
    
    pub fn create_ticker<F>(&mut self, on_tick: F) -> Ticker 
    where
        F: FnMut(Duration) -> TickerFuture + Send + 'static,
    {
        let ticker = Ticker::new(Box::new(on_tick));
        self.tickers.push(ticker.clone());
        ticker
    }
    
    pub fn dispose_all(&mut self) {
        for ticker in &mut self.tickers {
            ticker.dispose();
        }
        self.tickers.clear();
    }
    
    pub fn set_mounted(&mut self, mounted: bool) {
        self.mounted = mounted;
        
        // Pause/resume tickers based on mount state
        for ticker in &mut self.tickers {
            if !mounted && ticker.is_active() {
                ticker.stop(false);
            }
        }
    }
}

//=============================================================================
// Example State with SingleTickerProviderStateMixin
//=============================================================================

struct AnimatedWidgetState {
    // State fields
    controller: Option<AnimationController>,
    
    // Ticker mixin
    ticker_state: SingleTickerProviderState,
    
    // Required by State trait
    context: Option<BuildContext>,
}

impl AnimatedWidgetState {
    fn new() -> Self {
        Self {
            controller: None,
            ticker_state: SingleTickerProviderState::new(),
            context: None,
        }
    }
}

impl State for AnimatedWidgetState {
    type Widget = AnimatedWidget;
    
    fn init_state(&mut self) {
        // Create ticker
        let ctx = self.context.clone();
        let ticker = self.ticker_state.create_ticker(move |elapsed| {
            // Trigger rebuild on animation tick
            if let Some(context) = &ctx {
                context.mark_dirty();
            }
            TickerFuture::Continue
        });
        
        // Create AnimationController with vsync
        let controller = AnimationController::new(
            &SingleTickerProvider(ticker),
            Duration::from_millis(300)
        );
        
        // Add listener to rebuild on animation changes
        let ctx_clone = self.context.clone();
        controller.add_listener(Box::new(move || {
            if let Some(context) = &ctx_clone {
                context.mark_dirty();
            }
        }));
        
        self.controller = Some(controller);
        
        // Start animation
        if let Some(ctrl) = &mut self.controller {
            ctrl.forward(None);
        }
    }
    
    fn build(&mut self, _context: &BuildContext) -> Box<dyn Widget> {
        let opacity = self.controller.as_ref()
            .map(|c| c.value())
            .unwrap_or(0.0);
        
        Opacity::new(
            opacity,
            Container::new()
                .color(Color::BLUE)
                .child(Text::new("Animated!"))
        ).into_widget()
    }
    
    fn dispose(&mut self) {
        // Dispose controller
        if let Some(controller) = &mut self.controller {
            controller.dispose();
        }
        
        // Dispose ticker
        self.ticker_state.dispose_ticker();
    }
    
    fn mark_needs_build(&mut self) {
        if let Some(ctx) = &self.context {
            ctx.mark_dirty();
        }
    }
    
    fn context(&self) -> &BuildContext {
        self.context.as_ref().unwrap()
    }
    
    fn set_context(&mut self, context: BuildContext) {
        self.context = Some(context);
    }
}

// Wrapper for SingleTickerProvider
struct SingleTickerProvider(Ticker);

impl TickerProvider for SingleTickerProvider {
    fn create_ticker(&self, on_tick: TickerCallback) -> Ticker {
        self.0.clone()
    }
}

//=============================================================================
// AnimatedWidget - base class for animated widgets
//=============================================================================

/// AnimatedWidget - rebuilds when animation changes
pub trait AnimatedWidget: Widget {
    /// The animation to listen to
    fn animation(&self) -> &dyn Animation<f64>;
}

/// AnimatedBuilder - builds widget based on animation value
pub struct AnimatedBuilder {
    animation: Arc<Mutex<AnimationController>>,
    builder: Arc<dyn Fn(&BuildContext, f64, Option<Box<dyn Widget>>) -> Box<dyn Widget> + Send + Sync>,
    child: Option<Box<dyn Widget>>,
}

impl AnimatedBuilder {
    pub fn new<F>(
        animation: Arc<Mutex<AnimationController>>,
        builder: F,
    ) -> Self 
    where
        F: Fn(&BuildContext, f64, Option<Box<dyn Widget>>) -> Box<dyn Widget> + Send + Sync + 'static,
    {
        Self {
            animation,
            builder: Arc::new(builder),
            child: None,
        }
    }
    
    pub fn child(mut self, child: impl IntoWidget) -> Self {
        self.child = Some(child.into_widget());
        self
    }
}

impl StatefulWidget for AnimatedBuilder {
    type State = AnimatedBuilderState;
    
    fn create_state(&self) -> Self::State {
        AnimatedBuilderState {
            animation: self.animation.clone(),
            builder: self.builder.clone(),
            child: self.child.clone(),
            listener_id: None,
            context: None,
        }
    }
}

struct AnimatedBuilderState {
    animation: Arc<Mutex<AnimationController>>,
    builder: Arc<dyn Fn(&BuildContext, f64, Option<Box<dyn Widget>>) -> Box<dyn Widget> + Send + Sync>,
    child: Option<Box<dyn Widget>>,
    listener_id: Option<ListenerId>,
    context: Option<BuildContext>,
}

impl State for AnimatedBuilderState {
    type Widget = AnimatedBuilder;
    
    fn init_state(&mut self) {
        // Listen to animation changes
        let ctx = self.context.clone();
        let id = self.animation.lock().unwrap().add_listener(Box::new(move || {
            if let Some(context) = &ctx {
                context.mark_dirty();
            }
        }));
        self.listener_id = Some(id);
    }
    
    fn build(&mut self, context: &BuildContext) -> Box<dyn Widget> {
        let value = self.animation.lock().unwrap().value();
        (self.builder)(context, value, self.child.clone())
    }
    
    fn dispose(&mut self) {
        if let Some(id) = self.listener_id {
            self.animation.lock().unwrap().remove_listener(id);
        }
    }
    
    fn mark_needs_build(&mut self) {
        if let Some(ctx) = &self.context {
            ctx.mark_dirty();
        }
    }
    
    fn context(&self) -> &BuildContext {
        self.context.as_ref().unwrap()
    }
    
    fn set_context(&mut self, context: BuildContext) {
        self.context = Some(context);
    }
}

//=============================================================================
// Complete Usage Example
//=============================================================================

struct FadeInDemo;

impl StatefulWidget for FadeInDemo {
    type State = FadeInDemoState;
    
    fn create_state(&self) -> Self::State {
        FadeInDemoState::new()
    }
}

struct FadeInDemoState {
    controller: Option<AnimationController>,
    ticker_state: SingleTickerProviderState,
    context: Option<BuildContext>,
}

impl FadeInDemoState {
    fn new() -> Self {
        Self {
            controller: None,
            ticker_state: SingleTickerProviderState::new(),
            context: None,
        }
    }
}

impl State for FadeInDemoState {
    type Widget = FadeInDemo;
    
    fn init_state(&mut self) {
        // Create ticker
        let ctx = self.context.clone();
        let ticker = self.ticker_state.create_ticker(move |elapsed| {
            if let Some(context) = &ctx {
                context.mark_dirty();
            }
            TickerFuture::Continue
        });
        
        // Create controller
        let mut controller = AnimationController::new(
            &SingleTickerProvider(ticker),
            Duration::from_millis(1000)
        );
        
        // Add listener
        let ctx_clone = self.context.clone();
        controller.add_listener(Box::new(move || {
            if let Some(context) = &ctx_clone {
                context.mark_dirty();
            }
        }));
        
        // Start animation
        controller.forward(None);
        
        self.controller = Some(controller);
    }
    
    fn build(&mut self, context: &BuildContext) -> Box<dyn Widget> {
        let ctrl = self.controller.as_ref().unwrap();
        
        // Use AnimatedBuilder pattern
        AnimatedBuilder::new(
            Arc::new(Mutex::new(ctrl.clone())),
            |ctx, value, child| {
                Opacity::new(
                    value,
                    child.unwrap_or_else(|| Container::new().into_widget())
                ).into_widget()
            }
        )
        .child(
            Container::new()
                .width(200.0)
                .height(200.0)
                .color(Color::BLUE)
                .child(
                    Center::new(
                        Text::new("Fade In!")
                            .color(Color::WHITE)
                    )
                )
        )
        .into_widget()
    }
    
    fn dispose(&mut self) {
        if let Some(controller) = &mut self.controller {
            controller.dispose();
        }
        self.ticker_state.dispose_ticker();
    }
    
    fn mark_needs_build(&mut self) {
        if let Some(ctx) = &self.context {
            ctx.mark_dirty();
        }
    }
    
    fn context(&self) -> &BuildContext {
        self.context.as_ref().unwrap()
    }
    
    fn set_context(&mut self, context: BuildContext) {
        self.context = Some(context);
    }
}

//=============================================================================
// Alternative: Simplified API with macro
//=============================================================================

/// Macro to automatically implement ticker mixin
#[macro_export]
macro_rules! impl_single_ticker_provider {
    ($state:ty) => {
        impl SingleTickerProviderStateMixin for $state {
            fn ticker(&self) -> &Option<Ticker> {
                &self.ticker_state.ticker
            }
            
            fn ticker_mut(&mut self) -> &mut Option<Ticker> {
                &mut self.ticker_state.ticker
            }
            
            fn is_mounted(&self) -> bool {
                self.ticker_state.mounted
            }
        }
    };
}

// Usage:
// impl_single_ticker_provider!(MyAnimatedState);

//=============================================================================
// Integration with SchedulerBinding
//=============================================================================

/// SchedulerBinding - manages frame callbacks
pub struct SchedulerBinding {
    tickers: Vec<Weak<Mutex<Ticker>>>,
    frame_callbacks: Vec<Box<dyn FnMut(Duration) + Send>>,
}

impl SchedulerBinding {
    pub fn new() -> Self {
        Self {
            tickers: Vec::new(),
            frame_callbacks: Vec::new(),
        }
    }
    
    /// Register ticker
    pub fn add_ticker(&mut self, ticker: Weak<Mutex<Ticker>>) {
        self.tickers.push(ticker);
    }
    
    /// Schedule frame callback
    pub fn schedule_frame_callback(&mut self, callback: Box<dyn FnMut(Duration) + Send>) {
        self.frame_callbacks.push(callback);
    }
    
    /// Handle frame - called by platform integration
    pub fn handle_begin_frame(&mut self, frame_time: Duration) {
        // Tick all active tickers
        self.tickers.retain(|weak_ticker| {
            if let Some(ticker) = weak_ticker.upgrade() {
                let mut ticker = ticker.lock().unwrap();
                if ticker.is_active() {
                    ticker.tick();
                }
                true
            } else {
                false // Remove dead weak reference
            }
        });
        
        // Run frame callbacks
        for callback in &mut self.frame_callbacks {
            callback(frame_time);
        }
        self.frame_callbacks.clear();
    }
}

lazy_static! {
    pub static ref SCHEDULER: Mutex<SchedulerBinding> = Mutex::new(SchedulerBinding::new());
}
