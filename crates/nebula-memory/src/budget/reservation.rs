//! Memory reservation system for the budgeting system
//!
//! This module provides a memory reservation system that allows components
//! to reserve memory in advance, ensuring it will be available when needed.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::{MemoryError, MemoryResult};
use super::budget::MemoryBudget;
use super::config::ReservationMode;

/// Memory reservation for guaranteed memory availability
pub struct MemoryReservation {
    /// Budget this reservation is for
    budget: Arc<MemoryBudget>,
    
    /// Amount of memory reserved
    amount: usize,
    
    /// When the reservation was created
    created_at: Instant,
    
    /// Expiration time (if any)
    expires_at: Option<Instant>,
    
    /// Reservation mode
    mode: ReservationMode,
    
    /// Whether the reservation has been claimed
    claimed: Mutex<bool>,
    
    /// Whether the reservation has been canceled
    canceled: Mutex<bool>,
}

/// Token representing a claimed reservation
pub struct ReservationToken {
    /// The reservation this token is for
    reservation: Arc<MemoryReservation>,
    
    /// Whether the token has been released
    released: Mutex<bool>,
}

impl MemoryReservation {
    /// Create a new memory reservation
    pub fn new(
        budget: Arc<MemoryBudget>,
        amount: usize,
        mode: ReservationMode,
        ttl: Option<Duration>,
    ) -> MemoryResult<Arc<Self>> {
        // Check if the budget can allocate the requested amount
        if !budget.can_allocate(amount) {
            return Err(MemoryError::OutOfMemory {
                requested: amount,
                available: budget.limit() - budget.used(),
                context: format!("Cannot reserve {} bytes in budget '{}'", amount, budget.name()),
            });
        }
        
        // For strict reservations, actually allocate the memory now
        if mode == ReservationMode::Strict {
            budget.request_memory(amount)?;
        }
        
        let now = Instant::now();
        let expires_at = ttl.map(|ttl| now + ttl);
        
        let reservation = Arc::new(Self {
            budget,
            amount,
            created_at: now,
            expires_at,
            mode,
            claimed: Mutex::new(false),
            canceled: Mutex::new(false),
        });
        
        Ok(reservation)
    }
    
    /// Get the amount of memory reserved
    pub fn amount(&self) -> usize {
        self.amount
    }
    
    /// Get the budget this reservation is for
    pub fn budget(&self) -> &Arc<MemoryBudget> {
        &self.budget
    }
    
    /// Check if the reservation has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            Instant::now() > expires_at
        } else {
            false
        }
    }
    
    /// Check if the reservation has been claimed
    pub fn is_claimed(&self) -> bool {
        *self.claimed.lock().unwrap()
    }
    
    /// Check if the reservation has been canceled
    pub fn is_canceled(&self) -> bool {
        *self.canceled.lock().unwrap()
    }
    
    /// Claim the reservation
    pub fn claim(self: &Arc<Self>) -> MemoryResult<ReservationToken> {
        let mut claimed = self.claimed.lock().unwrap();
        let canceled = self.canceled.lock().unwrap();
        
        if *claimed {
            return Err(MemoryError::InvalidOperation {
                reason: "Reservation has already been claimed".to_string(),
            });
        }
        
        if *canceled {
            return Err(MemoryError::InvalidOperation {
                reason: "Reservation has been canceled".to_string(),
            });
        }
        
        if self.is_expired() {
            return Err(MemoryError::InvalidOperation {
                reason: "Reservation has expired".to_string(),
            });
        }
        
        // For non-strict reservations, allocate the memory now
        if self.mode != ReservationMode::Strict {
            self.budget.request_memory(self.amount)?;
        }
        
        *claimed = true;
        
        Ok(ReservationToken {
            reservation: self.clone(),
            released: Mutex::new(false),
        })
    }
    
    /// Cancel the reservation
    pub fn cancel(&self) -> MemoryResult<()> {
        let mut canceled = self.canceled.lock().unwrap();
        let claimed = self.claimed.lock().unwrap();
        
        if *claimed {
            return Err(MemoryError::InvalidOperation {
                reason: "Cannot cancel a claimed reservation".to_string(),
            });
        }
        
        if *canceled {
            return Err(MemoryError::InvalidOperation {
                reason: "Reservation has already been canceled".to_string(),
            });
        }
        
        // For strict reservations, release the memory
        if self.mode == ReservationMode::Strict {
            self.budget.release_memory(self.amount);
        }
        
        *canceled = true;
        
        Ok(())
    }
}

impl Drop for MemoryReservation {
    fn drop(&mut self) {
        // If the reservation is strict and hasn't been claimed or canceled,
        // release the memory
        if self.mode == ReservationMode::Strict
            && !*self.claimed.lock().unwrap()
            && !*self.canceled.lock().unwrap()
        {
            self.budget.release_memory(self.amount);
        }
    }
}

impl ReservationToken {
    /// Get the amount of memory reserved
    pub fn amount(&self) -> usize {
        self.reservation.amount
    }
    
    /// Get the budget this reservation is for
    pub fn budget(&self) -> &Arc<MemoryBudget> {
        &self.reservation.budget
    }
    
    /// Release the reserved memory
    pub fn release(&self) {
        let mut released = self.released.lock().unwrap();
        
        if !*released {
            self.reservation.budget.release_memory(self.reservation.amount);
            *released = true;
        }
    }
    
    /// Check if the token has been released
    pub fn is_released(&self) -> bool {
        *self.released.lock().unwrap()
    }
}

impl Drop for ReservationToken {
    fn drop(&mut self) {
        // Automatically release the memory if not already released
        if !*self.released.lock().unwrap() {
            self.reservation.budget.release_memory(self.reservation.amount);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::config::BudgetConfig;
    
    #[test]
    fn test_strict_reservation() {
        let budget = MemoryBudget::new(BudgetConfig::new("test", 1000));
        
        // Create a strict reservation
        let reservation = MemoryReservation::new(
            budget.clone(),
            500,
            ReservationMode::Strict,
            None,
        ).unwrap();
        
        // Memory should be allocated immediately
        assert_eq!(budget.used(), 500);
        
        // Claim the reservation
        let token = reservation.claim().unwrap();
        
        // Memory usage should not change (already allocated)
        assert_eq!(budget.used(), 500);
        
        // Release the token
        token.release();
        
        // Memory should be released
        assert_eq!(budget.used(), 0);
    }
    
    #[test]
    fn test_best_effort_reservation() {
        let budget = MemoryBudget::new(BudgetConfig::new("test", 1000));
        
        // Create a best-effort reservation
        let reservation = MemoryReservation::new(
            budget.clone(),
            500,
            ReservationMode::BestEffort,
            None,
        ).unwrap();
        
        // Memory should not be allocated yet
        assert_eq!(budget.used(), 0);
        
        // Claim the reservation
        let token = reservation.claim().unwrap();
        
        // Memory should be allocated now
        assert_eq!(budget.used(), 500);
        
        // Release the token
        token.release();
        
        // Memory should be released
        assert_eq!(budget.used(), 0);
    }
    
    #[test]
    fn test_reservation_expiration() {
        let budget = MemoryBudget::new(BudgetConfig::new("test", 1000));
        
        // Create a reservation with a very short TTL
        let reservation = MemoryReservation::new(
            budget.clone(),
            500,
            ReservationMode::BestEffort,
            Some(Duration::from_millis(1)),
        ).unwrap();
        
        // Wait for the reservation to expire
        std::thread::sleep(Duration::from_millis(10));
        
        // Reservation should be expired
        assert!(reservation.is_expired());
        
        // Claiming should fail
        assert!(reservation.claim().is_err());
    }
    
    #[test]
    fn test_reservation_cancellation() {
        let budget = MemoryBudget::new(BudgetConfig::new("test", 1000));
        
        // Create a strict reservation
        let reservation = MemoryReservation::new(
            budget.clone(),
            500,
            ReservationMode::Strict,
            None,
        ).unwrap();
        
        // Memory should be allocated immediately
        assert_eq!(budget.used(), 500);
        
        // Cancel the reservation
        reservation.cancel().unwrap();
        
        // Memory should be released
        assert_eq!(budget.used(), 0);
        
        // Claiming should fail
        assert!(reservation.claim().is_err());
    }
    
    #[test]
    fn test_automatic_release() {
        let budget = MemoryBudget::new(BudgetConfig::new("test", 1000));
        
        // Create a scope to test automatic release
        {
            // Create a strict reservation
            let reservation = MemoryReservation::new(
                budget.clone(),
                500,
                ReservationMode::Strict,
                None,
            ).unwrap();
            
            // Memory should be allocated immediately
            assert_eq!(budget.used(), 500);
            
            // Claim the reservation
            let token = reservation.claim().unwrap();
            
            // Memory usage should not change (already allocated)
            assert_eq!(budget.used(), 500);
            
            // Let the token go out of scope
        }
        
        // Memory should be released automatically
        assert_eq!(budget.used(), 0);
    }
}