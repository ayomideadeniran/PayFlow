#![no_std]

mod errors;
mod test;

use soroban_sdk::{
    contract, contractimpl, contracttype,
    token, Address, Env, Symbol,
};
use crate::errors::ContractError;

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Subscription(Address), // user → Subscription
    Token,                 // the XLM / token contract address
}

// ── Data types ────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Subscription {
    pub merchant: Address,
    pub amount: i128,       // amount per period (in stroops / smallest unit)
    pub interval: u64,      // seconds between charges
    pub last_charged: u64,  // ledger timestamp of last charge
    pub active: bool,
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct FlowPay;

#[contractimpl]
impl FlowPay {
    /// One-time initialisation: set the token contract this subscription
    /// manager will move (e.g. native XLM or a USDC SAC address).
    pub fn initialize(env: Env, token: Address) {
        if env.storage().instance().has(&DataKey::Token) {
            env.panic_with_error(ContractError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Token, &token);
    }

    /// User creates (or updates) a subscription to a merchant.
    ///
    /// The user must have already called `approve()` on the token contract
    /// granting this contract an allowance >= amount.
    pub fn subscribe(
        env: Env,
        user: Address,
        merchant: Address,
        amount: i128,
        interval: u64, // e.g. 2_592_000 for ~30 days
    ) {
        user.require_auth();

        if amount <= 0 {
            env.panic_with_error(ContractError::AmountMustBePositive);
        }
        if interval == 0 {
            env.panic_with_error(ContractError::IntervalMustBePositive);
        }

        let sub = Subscription {
            merchant,
            amount,
            interval,
            last_charged: env.ledger().timestamp(),
            active: true,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Subscription(user.clone()), &sub);

        env.events().publish(
            (Symbol::new(&env, "subscribed"), user),
            (sub.merchant, sub.amount, sub.interval),
        );
    }

    /// Charge a user's subscription.
    ///
    /// Anyone can call this (your backend / keeper service will call it).
    /// The contract verifies the interval has elapsed before transferring.
    pub fn charge(env: Env, user: Address) {
        let key = DataKey::Subscription(user.clone());

        let mut sub: Subscription = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| env.panic_with_error(ContractError::NoSubscriptionFound));

        if !sub.active {
            env.panic_with_error(ContractError::SubscriptionInactive);
        }

        let now = env.ledger().timestamp();
        if now < sub.last_charged + sub.interval {
            env.panic_with_error(ContractError::IntervalNotElapsed);
        }

        // Pull the token address stored at init
        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .unwrap_or_else(|| env.panic_with_error(ContractError::NotInitialized));

        // Transfer from user → merchant using the allowance the user granted
        let token = token::Client::new(&env, &token_addr);
        token.transfer_from(
            &env.current_contract_address(),
            &user,
            &sub.merchant,
            &sub.amount,
        );

        sub.last_charged = now;
        env.storage().persistent().set(&key, &sub);

        env.events().publish(
            (Symbol::new(&env, "charged"), user),
            (sub.merchant, sub.amount, now),
        );
    }

    /// Pay-per-use microtransaction — charge an arbitrary amount right now,
    /// no interval check. Useful for metered / usage-based billing.
    pub fn pay_per_use(env: Env, user: Address, amount: i128) {
        user.require_auth();

        if amount <= 0 {
            env.panic_with_error(ContractError::AmountMustBePositive);
        }

        let key = DataKey::Subscription(user.clone());
        let sub: Subscription = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| env.panic_with_error(ContractError::NoSubscriptionFound));

        if !sub.active {
            env.panic_with_error(ContractError::SubscriptionInactive);
        }

        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .unwrap_or_else(|| env.panic_with_error(ContractError::NotInitialized));

        let token = token::Client::new(&env, &token_addr);
        token.transfer_from(
            &env.current_contract_address(),
            &user,
            &sub.merchant,
            &amount,
        );

        env.events().publish(
            (Symbol::new(&env, "pay_per_use"), user),
            (sub.merchant, amount),
        );
    }

    /// Cancel a subscription. Only the subscriber can cancel.
    pub fn cancel(env: Env, user: Address) {
        user.require_auth();

        let key = DataKey::Subscription(user.clone());
        let mut sub: Subscription = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| env.panic_with_error(ContractError::NoSubscriptionFound));

        sub.active = false;
        env.storage().persistent().set(&key, &sub);

        env.events()
            .publish((Symbol::new(&env, "cancelled"), user), ());
    }

    /// Read a subscription (view function).
    pub fn get_subscription(env: Env, user: Address) -> Option<Subscription> {
        env.storage()
            .persistent()
            .get(&DataKey::Subscription(user))
    }
}
