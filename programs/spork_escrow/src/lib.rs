use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("G2Z1FrkEDxgsvoVi7a8C1P3gejdCbph9BRAh7rTCsb4B");

pub const CONFIG_SEED: &[u8] = b"spork";
pub const CONFIG_ITEM: &[u8] = b"config";
pub const ORDER_SEED: &[u8] = b"spork";
pub const ORDER_ITEM: &[u8] = b"order";
pub const VAULT_SEED: &[u8] = b"spork";
pub const VAULT_ITEM: &[u8] = b"vault";
pub const VAULT_AUTH_SEED: &[u8] = b"spork";
pub const VAULT_AUTH_ITEM: &[u8] = b"vault_authority";

pub const MAX_ORDER_ID_LEN: usize = 68;
pub const FEE_WALLET_CHANGE_DELAY_SECS: i64 = 7 * 24 * 60 * 60;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[error_code]
pub enum SporkError {
    #[msg("The signer is not the designated authority")]
    UnauthorizedAuthority,
    #[msg("The protocol is paused")]
    ProtocolPaused,
    #[msg("Customer token account fails the mint/owner constraint")]
    CustomerMintMismatch,
    #[msg("The order is not in a state that permits this action")]
    InvalidOrderState,
    #[msg("The fee-wall change timelock has not elapsed")]
    TimelockNotElapsed,
    #[msg("No fee-wallet change is pending")]
    NoPendingFeeWalletChange,
    #[msg("The fee-wallet change request is still pending")]
    FeeWalletChangePending,
    #[msg("Insufficient token balance to escrow")]
    InsufficientBalance,
}

// ---------------------------------------------------------------------------
// Global config
// ---------------------------------------------------------------------------

/// Global protocol config, PDA `["spork", "config"]`.
#[account]
pub struct Config {
    pub authority: Pubkey,
    pub arbiter: Pubkey,
    pub fee_wallet: Pubkey,
    pub pending_fee_wallet: Option<Pubkey>,
    pub fee_wallet_change_requested_at: i64,
    pub flat_fee_lamports: u64,
    pub paused: bool,
    pub bump: u8,
}

impl Config {
    pub const BASE_SIZE: usize = 8 // discriminator
        + 32 // authority
        + 32 // arbiter
        + 32 // fee_wallet
        + 1 + 32 // pending_fee_wallet Option
        + 8 // change requested at
        + 8 // flat fee
        + 1 // paused
        + 1 // bump
        + 7; // padding

    pub const SIZE: usize = Self::BASE_SIZE;

    pub fn init(
        &mut self,
        authority: Pubkey,
        arbiter: Pubkey,
        flat_fee_lamports: u64,
        paused: bool,
    ) -> Result<()> {
        self.authority = authority;
        self.arbiter = arbiter;
        self.fee_wallet = authority;
        self.pending_fee_wallet = None;
        self.fee_wallet_change_requested_at = 0;
        self.flat_fee_lamports = flat_fee_lamports;
        self.paused = paused;
        Ok(())
    }

    pub fn apply(
        &mut self,
        authority: Option<Pubkey>,
        arbiter: Option<Pubkey>,
        flat_fee_lamports: Option<u64>,
        paused: Option<bool>,
    ) -> Result<()> {
        if let Some(a) = authority {
            self.authority = a;
        }
        if let Some(a) = arbiter {
            self.arbiter = a;
        }
        if let Some(f) = flat_fee_lamports {
            self.flat_fee_lamports = f;
        }
        if let Some(p) = paused {
            self.paused = p;
        }
        Ok(())
    }

    pub fn request(&mut self, new_fee_wallet: Pubkey, now: i64) -> Result<()> {
        require!(!self.paused, SporkError::ProtocolPaused);
        require!(
            self.pending_fee_wallet.is_none(),
            SporkError::FeeWalletChangePending
        );
        self.pending_fee_wallet = Some(new_fee_wallet);
        self.fee_wallet_change_requested_at = now;
        Ok(())
    }

    pub fn execute(&mut self, now: i64) -> Result<()> {
        let pending = self
            .pending_fee_wallet
            .ok_or(SporkError::NoPendingFeeWalletChange)?;
        require!(
            now >= self.fee_wallet_change_requested_at + FEE_WALLET_CHANGE_DELAY_SECS,
            SporkError::TimelockNotElapsed
        );
        self.fee_wallet = pending;
        self.pending_fee_wallet = None;
        self.fee_wallet_change_requested_at = 0;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Order status
// ---------------------------------------------------------------------------

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum OrderStatus {
    Created,
    Preparing,
    ReadyForPickup,
    PickedUp,
    Delivered,
    Settled,
    Cancelled,
    Refunded,
    Disputed,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub struct OrderStatusWrapper(pub OrderStatus);

// ---------------------------------------------------------------------------
// Order
// ---------------------------------------------------------------------------

#[account]
pub struct Order {
    pub config: Pubkey,
    pub order_id: String,
    pub customer: Pubkey,
    pub merchant: Pubkey,
    pub driver: Option<Pubkey>,
    pub mint: Pubkey,
    pub amount_lamports: u64,
    pub fee_lamports: u64,
    pub status: OrderStatus,
    pub created_at: i64,
    pub accepted_at: i64,
    pub ready_at: i64,
    pub pickup_at: i64,
    pub delivery_at: i64,
    pub disputed_at: i64,
    pub bump: u8,
}

impl Order {
    pub const BASE_SIZE: usize = 8 // discriminator
        + 32 // config
        + 4 + MAX_ORDER_ID_LEN // order_id String
        + 32 // customer
        + 32 // merchant
        + 1 + 32 // driver Option
        + 32 // mint
        + 8 // amount
        + 8 // fee
        + 1 // status
        + 8 * 6 // timestamps
        + 1 // bump
        + 7; // padding

    pub fn size(order_id_len: usize) -> usize {
        Self::BASE_SIZE + order_id_len.saturating_sub(MAX_ORDER_ID_LEN)
    }

    pub fn create(
        &mut self,
        order_id: String,
        customer: Pubkey,
        merchant: Pubkey,
        mint: Pubkey,
        amount_lamports: u64,
        bump: u8,
    ) -> Result<()> {
        require!(order_id.len() <= MAX_ORDER_ID_LEN, SporkError::InvalidOrderState);
        self.order_id = order_id;
        self.customer = customer;
        self.merchant = merchant;
        self.driver = None;
        self.mint = mint;
        self.amount_lamports = amount_lamports;
        self.fee_lamports = self.fee_lamports;
        self.status = OrderStatus::Created;
        self.bump = bump;
        Ok(())
    }

    pub fn accept(&mut self, driver: Pubkey) -> Result<()> {
        require!(self.status == OrderStatus::Created, SporkError::InvalidOrderState);
        self.driver = Some(driver);
        self.status = OrderStatus::Preparing;
        Ok(())
    }

    pub fn mark_ready(&mut self) -> Result<()> {
        require!(self.status == OrderStatus::Preparing, SporkError::InvalidOrderState);
        self.status = OrderStatus::ReadyForPickup;
        Ok(())
    }

    pub fn confirm_pickup(&mut self) -> Result<()> {
        require!(self.status == OrderStatus::ReadyForPickup, SporkError::InvalidOrderState);
        self.status = OrderStatus::PickedUp;
        Ok(())
    }

    pub fn confirm_delivery(&mut self) -> Result<()> {
        require!(self.status == OrderStatus::PickedUp, SporkError::InvalidOrderState);
        self.status = OrderStatus::Delivered;
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<()> {
        require!(self.status == OrderStatus::Created, SporkError::InvalidOrderState);
        self.status = OrderStatus::Cancelled;
        Ok(())
    }

    pub fn timeout_refund(&mut self) -> Result<()> {
        require!(
            self.status == OrderStatus::Created || self.status == OrderStatus::Preparing,
            SporkError::InvalidOrderState
        );
        self.status = OrderStatus::Refunded;
        Ok(())
    }

    pub fn timeout_settle(&mut self) -> Result<()> {
        require!(self.status == OrderStatus::Delivered, SporkError::InvalidOrderState);
        self.status = OrderStatus::Settled;
        Ok(())
    }

    pub fn open_dispute(&mut self) -> Result<()> {
        require!(
            self.status == OrderStatus::Delivered || self.status == OrderStatus::PickedUp,
            SporkError::InvalidOrderState
        );
        self.status = OrderStatus::Disputed;
        Ok(())
    }

    pub fn resolve(&mut self, settle_to_merchant: bool) -> Result<()> {
        require!(self.status == OrderStatus::Disputed, SporkError::InvalidOrderState);
        self.status = if settle_to_merchant {
            OrderStatus::Settled
        } else {
            OrderStatus::Refunded
        };
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Entrypoints
// ---------------------------------------------------------------------------

#[program]
pub mod spork_escrow {
    use super::*;

    pub fn init_config(
        ctx: Context<InitConfig>,
        authority: Pubkey,
        arbiter: Pubkey,
        flat_fee_lamports: u64,
    ) -> Result<()> {
        ctx.accounts
            .config
            .init(authority, arbiter, flat_fee_lamports, false)
    }

    pub fn update_config(
        ctx: Context<UpdateConfig>,
        authority: Option<Pubkey>,
        arbiter: Option<Pubkey>,
        flat_fee_lamports: Option<u64>,
        paused: Option<bool>,
    ) -> Result<()> {
        ctx.accounts
            .apply(authority, arbiter, flat_fee_lamports, paused)
    }

    pub fn request_fee_wallet_change(ctx: Context<RequestFeeWalletChange>, new_fee_wallet: Pubkey) -> Result<()> {
        let now = ctx.accounts.clock.unix_timestamp;
        ctx.accounts.config.request(new_fee_wallet, now)
    }

    pub fn execute_fee_wallet_change(ctx: Context<ExecuteFeeWalletChange>) -> Result<()> {
        let now = ctx.accounts.clock.unix_timestamp;
        ctx.accounts.config.execute(now)
    }

    pub fn create_order(
        ctx: Context<CreateOrder>,
        order_id: String,
        customer: Pubkey,
        merchant: Pubkey,
        mint: Pubkey,
        amount_lamports: u64,
    ) -> Result<()> {
        ctx.accounts
            .create(order_id, customer, merchant, mint, amount_lamports)
    }

    pub fn accept_order(ctx: Context<AcceptOrder>, _order_id: String) -> Result<()> {
        ctx.accounts.order.accept(ctx.accounts.driver.key())
    }

    pub fn mark_ready(ctx: Context<MarkReady>, _order_id: String) -> Result<()> {
        ctx.accounts.order.mark_ready()
    }

    pub fn confirm_pickup(ctx: Context<ConfirmPickup>, _order_id: String) -> Result<()> {
        ctx.accounts.order.confirm_pickup()
    }

    pub fn confirm_delivery(ctx: Context<ConfirmDelivery>, _order_id: String) -> Result<()> {
        ctx.accounts.order.confirm_delivery()
    }

    pub fn cancel_order(ctx: Context<CancelOrder>, _order_id: String) -> Result<()> {
        ctx.accounts.order.cancel()
    }

    pub fn timeout_refund(ctx: Context<TimeoutRefund>, _order_id: String) -> Result<()> {
        ctx.accounts.order.timeout_refund()
    }

    pub fn timeout_settle(ctx: Context<TimeoutSettle>, _order_id: String) -> Result<()> {
        ctx.accounts.order.timeout_settle()
    }

    pub fn open_dispute(ctx: Context<OpenDispute>, _order_id: String) -> Result<()> {
        ctx.accounts.order.open_dispute()
    }

    pub fn resolve_dispute(
        ctx: Context<ResolveDispute>,
        _order_id: String,
        settle_to_merchant: bool,
    ) -> Result<()> {
        ctx.accounts.order.resolve(settle_to_merchant)
    }
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitConfig<'info> {
    #[account(
        init,
        payer = payer,
        space = Config::SIZE,
        seeds = [b"spork", b"config"],
        bump
    )]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"config"],
        bump,
        has_one = authority @ SporkError::UnauthorizedAuthority
    )]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct RequestFeeWalletChange<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"config"],
        bump,
        has_one = authority @ SporkError::UnauthorizedAuthority
    )]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
pub struct ExecuteFeeWalletChange<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"config"],
        bump,
        has_one = authority @ SporkError::UnauthorizedAuthority
    )]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
#[instruction(order_id: String)]
pub struct CreateOrder<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"config"],
        bump,
        constraint = !config.paused @ SporkError::ProtocolPaused,
    )]
    pub config: Account<'info, Config>,
    #[account(
        init,
        payer = payer,
        space = Order::size(order_id.len()),
        seeds = [b"spork", b"order", order_id.as_bytes()],
        bump,
    )]
    pub order: Account<'info, Order>,
    #[account(
        init,
        payer = payer,
        token::mint = mint,
        token::authority = vault_authority,
        seeds = [b"spork", b"vault", order_id.as_bytes()],
        bump,
    )]
    pub vault: Account<'info, TokenAccount>,
    /// PDA that signs for the vault authority.
    /// CHECK: seeds-only signer, never deserialized.
    #[account(
        seeds = [b"spork", b"vault_authority", order_id.as_bytes()],
        bump,
    )]
    pub vault_authority: AccountInfo<'info>,
    #[account(
        mut,
        constraint = customer_token.owner == customer.key() @ SporkError::CustomerMintMismatch,
        constraint = customer_token.mint == mint.key() @ SporkError::CustomerMintMismatch,
        constraint = customer_token.amount >= amount_lamports @ SporkError::InsufficientBalance,
    )]
    pub customer_token: Account<'info, TokenAccount>,
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: customer authority owner of `customer_token`.
    pub customer: AccountInfo<'info>,
    /// CHECK: receives the flat protocol fee in SOL.
    #[account(mut)]
    pub fee_wallet: AccountInfo<'info>,
    pub mint: Account<'info, Mint>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
#[instruction(order_id: String)]
pub struct AcceptOrder<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"order", order_id.as_bytes()],
        bump,
    )]
    pub order: Account<'info, Order>,
    /// CHECK: merchant or driver performing the accept.
    pub driver: Signer<'info>,
    #[account(mut)]
    pub config: Account<'info, Config>,
}

#[derive(Accounts)]
#[instruction(order_id: String)]
pub struct MarkReady<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"order", order_id.as_bytes()],
        bump,
    )]
    pub order: Account<'info, Order>,
    #[account(mut)]
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(order_id: String)]
pub struct ConfirmPickup<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"order", order_id.as_bytes()],
        bump,
    )]
    pub order: Account<'info, Order>,
    #[account(mut)]
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(order_id: String)]
pub struct ConfirmDelivery<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"order", order_id.as_bytes()],
        bump,
    )]
    pub order: Account<'info, Order>,
    #[account(mut)]
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(order_id: String)]
pub struct CancelOrder<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"order", order_id.as_bytes()],
        bump,
    )]
    pub order: Account<'info, Order>,
    #[account(mut)]
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(order_id: String)]
pub struct TimeoutRefund<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"order", order_id.as_bytes()],
        bump,
    )]
    pub order: Account<'info, Order>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
#[instruction(order_id: String)]
pub struct TimeoutSettle<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"order", order_id.as_bytes()],
        bump,
    )]
    pub order: Account<'info, Order>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
#[instruction(order_id: String)]
pub struct OpenDispute<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"order", order_id.as_bytes()],
        bump,
    )]
    pub order: Account<'info, Order>,
    #[account(mut)]
    pub customer: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(order_id: String)]
pub struct ResolveDispute<'info> {
    #[account(
        mut,
        seeds = [b"spork", b"order", order_id.as_bytes()],
        bump,
    )]
    pub order: Account<'info, Order>,
    #[account(mut)]
    pub authority: Signer<'info>,
}
