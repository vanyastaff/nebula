// nebula_animation/src/controller.rs

use nebula_foundation::*;
use nebula_scheduler::*;
use std::time::{Duration, Instant};

//=============================================================================
// Animation Trait - Base for all animations
//=============================================================================

/// Animation<T> - value that changes over time
pub trait Animation<T>: Send + Sync {
    /// Current value
    fn value(&self) -> T;
    
    /// Current status
    fn status(&self) -> AnimationStatus;
    
    /// Add value listener
    fn add_listener(&mut self, listener: VoidCallback) -> ListenerId;
    
    /// Remove value listener
    fn remove_listener(&mut self, id: ListenerId);
    
    /// Add status listener
    fn add_status_listener(&mut self, listener: StatusListener) -> ListenerId;
    
    /// Remove status listener
    fn remove_status_listener(&mut self, id: ListenerId);
    
    /// Is this animation dismissed?
    fn is_dismissed(&self) -> bool {
        self.status() == AnimationStatus::Dismissed
    }
    
    /// Is this animation completed?
    fn is_completed(&self) -> bool {
        self.status() == AnimationStatus::Completed
    }
}

/// Status listener callback
pub type StatusListener = Box<dyn Fn(AnimationStatus) + Send + Sync>;

/// Animation status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationStatus {
    /// Animation is stopped at the beginning
    Dismissed,
    /// Animation is running forward
    Forward,
    /// Animation is running in reverse
    Reverse,
    /// Animation is stopped at the end
    Completed,
}

//=============================================================================
// AnimationController - like Flutter, uses listener mixins pattern
//=============================================================================

/// AnimationController - drives animations
/// Uses AnimationLocalListenersMixin + AnimationLocalStatusListenersMixin pattern
pub struct AnimationController {
    // Value state
    value: f64,
    lower_bound: f64,
    upper_bound: f64,
    
    // Animation config
    duration: Duration,
    reverse_duration: Option<Duration>,
    
    // Status
    status: AnimationStatus,
    
    // Ticker for frame callbacks
    ticker: Option<Ticker>,
    
    // Listeners (like Flutter's mixins)
    value_listeners: ObserverList<VoidCallback>,
    status_listeners: ObserverList<StatusListener>,
    
    // Lifecycle
    disposed: bool,
    
    // Simulation tracking
    simulation: Option<Box<dyn Simulation>>,
    last_elapsed_duration: Option<Duration>,
}

impl AnimationController {
    /// Create new controller
    pub fn new(vsync: &dyn TickerProvider, duration: Duration) -> Self {
        Self {
            value: 0.0,
            lower_bound: 0.0,
            upper_bound: 1.0,
            duration,
            reverse_duration: None,
            status: AnimationStatus::Dismissed,
            ticker: Some(vsync.create_ticker(Box::new(|_| {}))),
            value_listeners: ObserverList::new(),
            status_listeners: ObserverList::new(),
            disposed: false,
            simulation: None,
            last_elapsed_duration: None,
        }
    }
    
    /// Create with custom bounds
    pub fn with_bounds(
        vsync: &dyn TickerProvider,
        duration: Duration,
        lower_bound: f64,
        upper_bound: f64,
    ) -> Self {
        assert!(lower_bound <= upper_bound, "lower_bound must be <= upper_bound");
        
        let mut controller = Self::new(vsync, duration);
        controller.lower_bound = lower_bound;
        controller.upper_bound = upper_bound;
        controller
    }
    
    /// Unbounded controller (no limits)
    pub fn unbounded(vsync: &dyn TickerProvider, duration: Duration) -> Self {
        Self::with_bounds(vsync, duration, f64::NEG_INFINITY, f64::INFINITY)
    }
    
    //=========================================================================
    // Control methods
    //=========================================================================
    
    /// Start animation forward
    pub fn forward(&mut self, from: Option<f64>) -> TickerFuture {
        self.assert_not_disposed();
        
        if let Some(from_value) = from {
            self.value = from_value.clamp(self.lower_bound, self.upper_bound);
        }
        
        self.status = AnimationStatus::Forward;
        self.notify_status_listeners();
        
        self.animate_to_internal(self.upper_bound, self.duration)
    }
    
    /// Start animation in reverse
    pub fn reverse(&mut self, from: Option<f64>) -> TickerFuture {
        self.assert_not_disposed();
        
        if let Some(from_value) = from {
            self.value = from_value.clamp(self.lower_bound, self.upper_bound);
        }
        
        self.status = AnimationStatus::Reverse;
        self.notify_status_listeners();
        
        let duration = self.reverse_duration.unwrap_or(self.duration);
        self.animate_to_internal(self.lower_bound, duration)
    }
    
    /// Animate to specific value
    pub fn animate_to(&mut self, target: f64, duration: Option<Duration>) -> TickerFuture {
        self.assert_not_disposed();
        
        let duration = duration.unwrap_or(self.duration);
        
        // Determine direction
        self.status = if target > self.value {
            AnimationStatus::Forward
        } else {
            AnimationStatus::Reverse
        };
        self.notify_status_listeners();
        
        self.animate_to_internal(target, duration)
    }
    
    /// Stop animation
    pub fn stop(&mut self, canceled: bool) {
        self.assert_not_disposed();
        
        self.simulation = None;
        self.last_elapsed_duration = None;
        
        if let Some(ticker) = &mut self.ticker {
            ticker.stop(canceled);
        }
    }
    
    /// Reset to beginning
    pub fn reset(&mut self) {
        self.value = self.lower_bound;
        self.status = AnimationStatus::Dismissed;
        self.notify_listeners();
        self.notify_status_listeners();
    }
    
    /// Repeat animation
    pub fn repeat(&mut self, min: Option<f64>, max: Option<f64>, reverse: bool, period: Option<Duration>) {
        let min = min.unwrap_or(self.lower_bound);
        let max = max.unwrap_or(self.upper_bound);
        let period = period.unwrap_or(self.duration);
        
        // Start repeating simulation
        self.animate_with_simulation(Box::new(RepeatSimulation::new(
            min, max, period, reverse,
        )));
    }
    
    //=========================================================================
    // Internal animation logic
    //=========================================================================
    
    fn animate_to_internal(&mut self, target: f64, duration: Duration) -> TickerFuture {
        let start_value = self.value;
        let start_time = Instant::now();
        
        // Create ticker callback
        let ticker = self.ticker.as_mut().unwrap();
        
        ticker.start(Box::new(move |elapsed| {
            let t = (elapsed.as_secs_f64() / duration.as_secs_f64()).min(1.0);
            
            // Linear interpolation
            self.value = start_value + (target - start_value) * t;
            self.notify_listeners();
            
            if t >= 1.0 {
                // Animation complete
                self.status = if target == self.upper_bound {
                    AnimationStatus::Completed
                } else {
                    AnimationStatus::Dismissed
                };
                self.notify_status_listeners();
                
                TickerFuture::Complete
            } else {
                TickerFuture::Continue
            }
        }))
    }
    
    fn animate_with_simulation(&mut self, simulation: Box<dyn Simulation>) {
        self.simulation = Some(simulation);
        self.last_elapsed_duration = Some(Duration::ZERO);
        
        let ticker = self.ticker.as_mut().unwrap();
        ticker.start(Box::new(move |elapsed| {
            if let Some(sim) = &self.simulation {
                self.value = sim.x(elapsed);
                self.notify_listeners();
                
                if sim.is_done(elapsed) {
                    self.status = AnimationStatus::Completed;
                    self.notify_status_listeners();
                    self.simulation = None;
                    return TickerFuture::Complete;
                }
            }
            
            TickerFuture::Continue
        }))
    }
    
    //=========================================================================
    // Listener management (like Flutter's AnimationLocalListenersMixin)
    //=========================================================================
    
    /// Notify all value listeners
    fn notify_listeners(&self) {
        if self.disposed {
            return;
        }
        
        // Create list of listeners to call (to avoid borrow issues)
        let listeners: Vec<_> = self.value_listeners.iter()
            .map(|cb| cb.clone())
            .collect();
        
        for listener in listeners {
            listener();
        }
    }
    
    /// Notify all status listeners (like AnimationLocalStatusListenersMixin)
    fn notify_status_listeners(&self) {
        if self.disposed {
            return;
        }
        
        let listeners: Vec<_> = self.status_listeners.iter()
            .map(|cb| cb.clone())
            .collect();
        
        for listener in listeners {
            listener(self.status);
        }
    }
    
    //=========================================================================
    // Lifecycle
    //=========================================================================
    
    pub fn dispose(&mut self) {
        assert!(!self.disposed, "AnimationController.dispose() called more than once");
        
        if let Some(ticker) = &mut self.ticker {
            ticker.dispose();
        }
        
        self.value_listeners = ObserverList::new();
        self.status_listeners = ObserverList::new();
        self.disposed = true;
    }
    
    fn assert_not_disposed(&self) {
        assert!(!self.disposed, "AnimationController was used after being disposed");
    }
    
    //=========================================================================
    // Getters
    //=========================================================================
    
    pub fn value(&self) -> f64 {
        self.value
    }
    
    pub fn status(&self) -> AnimationStatus {
        self.status
    }
    
    pub fn is_animating(&self) -> bool {
        self.ticker.as_ref()
            .map(|t| t.is_active())
            .unwrap_or(false)
    }
    
    pub fn velocity(&self) -> f64 {
        if let Some(sim) = &self.simulation {
            if let Some(elapsed) = self.last_elapsed_duration {
                return sim.dx(elapsed);
            }
        }
        0.0
    }
}

//=============================================================================
// Implement Animation<f64> for AnimationController
//=============================================================================

impl Animation<f64> for AnimationController {
    fn value(&self) -> f64 {
        self.value
    }
    
    fn status(&self) -> AnimationStatus {
        self.status
    }
    
    fn add_listener(&mut self, listener: VoidCallback) -> ListenerId {
        self.assert_not_disposed();
        self.value_listeners.add(listener)
    }
    
    fn remove_listener(&mut self, id: ListenerId) {
        self.value_listeners.remove(id);
    }
    
    fn add_status_listener(&mut self, listener: StatusListener) -> ListenerId {
        self.assert_not_disposed();
        self.status_listeners.add(listener)
    }
    
    fn remove_status_listener(&mut self, id: ListenerId) {
        self.status_listeners.remove(id);
    }
}

//=============================================================================
// Simulation trait
//=============================================================================

/// Simulation - physics-based animation
pub trait Simulation: Send + Sync {
    /// Position at time t
    fn x(&self, time: Duration) -> f64;
    
    /// Velocity at time t
    fn dx(&self, time: Duration) -> f64;
    
    /// Is simulation done?
    fn is_done(&self, time: Duration) -> bool;
}

/// Repeat simulation
struct RepeatSimulation {
    min: f64,
    max: f64,
    period: Duration,
    reverse: bool,
}

impl RepeatSimulation {
    fn new(min: f64, max: f64, period: Duration, reverse: bool) -> Self {
        Self { min, max, period, reverse }
    }
}

impl Simulation for RepeatSimulation {
    fn x(&self, time: Duration) -> f64 {
        let t = (time.as_secs_f64() % self.period.as_secs_f64()) / self.period.as_secs_f64();
        
        let t = if self.reverse {
            if t < 0.5 {
                t * 2.0
            } else {
                2.0 - t * 2.0
            }
        } else {
            t
        };
        
        self.min + (self.max - self.min) * t
    }
    
    fn dx(&self, _time: Duration) -> f64 {
        (self.max - self.min) / self.period.as_secs_f64()
    }
    
    fn is_done(&self, _time: Duration) -> bool {
        false // Never done
    }
}

//=============================================================================
// TickerProvider trait (like Flutter)
//=============================================================================

pub trait TickerProvider {
    /// Create a ticker
    fn create_ticker(&self, on_tick: TickerCallback) -> Ticker;
}

pub type TickerCallback = Box<dyn FnMut(Duration) -> TickerFuture + Send>;

//=============================================================================
// Ticker - frame callback (simplified)
//=============================================================================

pub struct Ticker {
    callback: Option<TickerCallback>,
    start_time: Option<Instant>,
    active: bool,
}

pub enum TickerFuture {
    Continue,
    Complete,
}

impl Ticker {
    pub fn new(callback: TickerCallback) -> Self {
        Self {
            callback: Some(callback),
            start_time: None,
            active: false,
        }
    }
    
    pub fn start(&mut self, callback: TickerCallback) -> TickerFuture {
        self.callback = Some(callback);
        self.start_time = Some(Instant::now());
        self.active = true;
        TickerFuture::Continue
    }
    
    pub fn tick(&mut self) -> Option<TickerFuture> {
        if !self.active {
            return None;
        }
        
        let elapsed = self.start_time.unwrap().elapsed();
        
        if let Some(callback) = &mut self.callback {
            Some(callback(elapsed))
        } else {
            None
        }
    }
    
    pub fn stop(&mut self, _canceled: bool) {
        self.active = false;
    }
    
    pub fn is_active(&self) -> bool {
        self.active
    }
    
    pub fn dispose(&mut self) {
        self.stop(true);
        self.callback = None;
    }
}

//=============================================================================
// Usage Example
//=============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    
    struct TestTickerProvider;
    
    impl TickerProvider for TestTickerProvider {
        fn create_ticker(&self, on_tick: TickerCallback) -> Ticker {
            Ticker::new(on_tick)
        }
    }
    
    #[test]
    fn test_animation_controller() {
        let vsync = TestTickerProvider;
        let mut controller = AnimationController::new(
            &vsync,
            Duration::from_millis(300)
        );
        
        // Add listener
        let id = controller.add_listener(Box::new(|| {
            println!("Value changed: {}", controller.value());
        }));
        
        // Add status listener
        controller.add_status_listener(Box::new(|status| {
            println!("Status changed: {:?}", status);
        }));
        
        // Start animation
        controller.forward(None);
        
        // Simulate ticks
        for _ in 0..10 {
            if let Some(ticker) = &mut controller.ticker {
                if let Some(TickerFuture::Complete) = ticker.tick() {
                    break;
                }
            }
        }
        
        assert_eq!(controller.status(), AnimationStatus::Completed);
        
        // Cleanup
        controller.remove_listener(id);
        controller.dispose();
    }
}
