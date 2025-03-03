use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, TokenAccount, Token, MintTo, Transfer};

declare_id!("DRjNVFEBb6NmJJmfFJgbQo64gYWCcsw2ibzvm7F9HXRQ");

#[program]
pub mod liquidity_acceleration_token {
    use super::*;

    /// Initializes the global program state.
    /// In addition to setting the base reward rates and PDAs, it also initializes
    /// parameters for the epoch-based reward system and liquidity pool boost.
    pub fn initialize(
        ctx: Context<Initialize>,
        trade_reward_rate: u64,
        stake_reward_rate: u64,
        trade_epoch_duration: i64,       // Duration (in seconds) for a trade reward epoch.
        pool_volume_threshold: u64,      // Threshold to trigger LP boost.
        pool_boost_multiplier: u64,      // Boost multiplier (percentage) for staking rewards.
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        state.admin = *ctx.accounts.admin.key;
        state.lat_mint = ctx.accounts.lat_mint.key();
        state.trade_reward_rate = trade_reward_rate;
        state.stake_reward_rate = stake_reward_rate;
        state.total_trades = 0;
        state.mint_auth_bump = ctx.bumps.mint_authority;
        state.vault_auth_bump = ctx.bumps.vault_authority;

        // Initialize new fields.
        state.epoch_trade_volume = 0;
        state.trade_epoch_duration = trade_epoch_duration;
        state.pool_trading_volume = 0;
        state.pool_volume_threshold = pool_volume_threshold;
        state.pool_boost_multiplier = pool_boost_multiplier;

        Ok(())
    }

    /// Records a trade by updating the trader's statistics and pending trade rewards.
    /// Rewards are calculated dynamically: if the global epoch trade volume is below a threshold,
    /// a higher multiplier is applied to encourage early activity.
    pub fn record_trade(ctx: Context<RecordTrade>, trade_volume: u64) -> Result<()> {
        let state = &mut ctx.accounts.state;
        state.total_trades = state.total_trades.checked_add(1).ok_or(ErrorCode::CalculationError)?;

        // Update global trade volume for the epoch and for the pool.
        state.epoch_trade_volume = state.epoch_trade_volume.checked_add(trade_volume).ok_or(ErrorCode::CalculationError)?;
        state.pool_trading_volume = state.pool_trading_volume.checked_add(trade_volume).ok_or(ErrorCode::CalculationError)?;

        let stats = &mut ctx.accounts.trader_stats;
        stats.trade_count = stats.trade_count.checked_add(1).ok_or(ErrorCode::CalculationError)?;
        stats.total_volume = stats.total_volume.checked_add(trade_volume).ok_or(ErrorCode::CalculationError)?;

        // Dynamic reward scaling:
        // If epoch trade volume is below the pool volume threshold, use a higher multiplier.
        const HIGH_REWARD_MULTIPLIER: u64 = 150; // 150% (for lower volume periods)
        const BASE_MULTIPLIER: u64 = 100;        // 100% (base rate)
        let multiplier = if state.epoch_trade_volume < state.pool_volume_threshold {
            HIGH_REWARD_MULTIPLIER
        } else {
            BASE_MULTIPLIER
        };

        let reward = trade_volume
            .checked_mul(state.trade_reward_rate)
            .and_then(|r| r.checked_mul(multiplier))
            .and_then(|r| r.checked_div(100))
            .ok_or(ErrorCode::CalculationError)?;

        // Instead of immediate minting, update the pending trade rewards counter.
        stats.pending_trade_rewards = stats.pending_trade_rewards.checked_add(reward).ok_or(ErrorCode::CalculationError)?;

        // Initialize the last claim timestamp if this is the first trade.
        if stats.last_claim == 0 {
            stats.last_claim = Clock::get()?.unix_timestamp;
        }

        Ok(())
    }

    /// Allows traders to claim their accumulated trade rewards after an epoch has ended.
    pub fn claim_trade_rewards(ctx: Context<ClaimTradeRewards>) -> Result<()> {
        let stats = &mut ctx.accounts.trader_stats;
        let current_time = Clock::get()?.unix_timestamp;

        // Ensure the epoch duration has passed.
        if current_time.checked_sub(stats.last_claim).unwrap() < ctx.accounts.state.trade_epoch_duration {
            return Err(ErrorCode::EpochNotEnded.into());
        }

        let reward = stats.pending_trade_rewards;
        if reward == 0 {
            return Err(ErrorCode::NoPendingRewards.into());
        }

        // Reset pending rewards and update the last claim timestamp.
        stats.pending_trade_rewards = 0;
        stats.last_claim = current_time;

        // Bind the state key to extend its lifetime.
        let state = &ctx.accounts.state;
        let state_key = state.key();
        let seeds = &[b"lat_mint_auth", state_key.as_ref(), &[state.mint_auth_bump]];
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.lat_mint.to_account_info(),
                    to: ctx.accounts.trader_token_account.to_account_info(),
                    authority: ctx.accounts.mint_authority.to_account_info(),
                },
                &[&seeds[..]],
            ),
            reward,
        )?;

        Ok(())
    }

    /// Stake LAT tokens by transferring them into the protocol's vault.
    /// This function also sets a 7-day vesting period before staking rewards can be claimed.
    pub fn stake_lat(ctx: Context<StakeLat>, amount: u64) -> Result<()> {
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.trader_token_account.to_account_info(),
                    to: ctx.accounts.staking_vault.to_account_info(),
                    authority: ctx.accounts.trader.to_account_info(),
                },
            ),
            amount,
        )?;

        let stake = &mut ctx.accounts.stake;
        // If first time staking, set the stake start time.
        if stake.stake_start == 0 {
            stake.stake_start = Clock::get()?.unix_timestamp;
        }
        stake.amount = stake.amount.checked_add(amount).ok_or(ErrorCode::CalculationError)?;
        stake.last_updated = Clock::get()?.unix_timestamp;
        Ok(())
    }

    /// Claim staking rewards based on the staked amount and the time elapsed.
    /// Enforces a 7-day vesting period (604800 seconds) before rewards can be claimed.
    /// Also applies a liquidity pool boost if the pool's trading volume exceeds a threshold.
    pub fn claim_stake_rewards(ctx: Context<ClaimRewards>) -> Result<()> {
        let stake = &mut ctx.accounts.stake;
        let current_time = Clock::get()?.unix_timestamp;

        // Check vesting period for flash loan protection.
        if current_time < stake.stake_start.checked_add(604800).ok_or(ErrorCode::CalculationError)? {
            return Err(ErrorCode::VestingPeriodNotCompleted.into());
        }

        let duration = current_time.checked_sub(stake.last_updated).ok_or(ErrorCode::CalculationError)? as u64;
        let state = &ctx.accounts.state;

        // Apply pool boost if the pool trading volume exceeds the threshold.
        let effective_stake_reward_rate = if state.pool_trading_volume > state.pool_volume_threshold {
            state.stake_reward_rate
                .checked_mul(state.pool_boost_multiplier)
                .and_then(|r| r.checked_div(100))
                .ok_or(ErrorCode::CalculationError)?
        } else {
            state.stake_reward_rate
        };

        let reward = stake.amount
            .checked_mul(effective_stake_reward_rate)
            .and_then(|r| r.checked_mul(duration))
            .ok_or(ErrorCode::CalculationError)?;

        stake.last_updated = current_time;

        let state_key = state.key();
        let seeds = &[b"lat_mint_auth", state_key.as_ref(), &[state.mint_auth_bump]];
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.lat_mint.to_account_info(),
                    to: ctx.accounts.trader_token_account.to_account_info(),
                    authority: ctx.accounts.mint_authority.to_account_info(),
                },
                &[&seeds[..]],
            ),
            reward,
        )?;

        Ok(())
    }

    /// Withdraw staked LAT tokens from the vault.
    pub fn withdraw_stake(ctx: Context<WithdrawStake>, amount: u64) -> Result<()> {
        let stake = &mut ctx.accounts.stake;
        require!(amount <= stake.amount, ErrorCode::InsufficientStake);
        stake.amount = stake.amount.checked_sub(amount).ok_or(ErrorCode::CalculationError)?;

        let state_key = ctx.accounts.state.key();
        let seeds = &[b"vault_auth", state_key.as_ref(), &[ctx.accounts.state.vault_auth_bump]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.staking_vault.to_account_info(),
                    to: ctx.accounts.trader_token_account.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                &[&seeds[..]],
            ),
            amount,
        )?;

        Ok(())
    }
}

/// Accounts for initializing the program state.
#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = admin, space = 8 + ProgramState::SIZE)]
    pub state: Account<'info, ProgramState>,
    #[account(mut)]
    pub admin: Signer<'info>,
    /// The LAT mint should have its authority set to the PDA derived from [b"lat_mint_auth", state.key()]
    #[account(mut)]
    pub lat_mint: Account<'info, Mint>,
    /// CHECK: PDA authority for minting LAT tokens.
    #[account(seeds = [b"lat_mint_auth", state.key().as_ref()], bump)]
    pub mint_authority: UncheckedAccount<'info>,
    /// CHECK: PDA authority for the staking vault.
    #[account(seeds = [b"vault_auth", state.key().as_ref()], bump)]
    pub vault_authority: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

/// Global state for the program.
#[account]
pub struct ProgramState {
    pub admin: Pubkey,
    pub lat_mint: Pubkey,
    pub trade_reward_rate: u64,
    pub stake_reward_rate: u64,
    pub total_trades: u64,
    pub mint_auth_bump: u8,
    pub vault_auth_bump: u8,
    // New fields for managing trade reward epochs and liquidity pool dynamics.
    pub epoch_trade_volume: u64,
    pub trade_epoch_duration: i64,
    pub pool_trading_volume: u64,
    pub pool_volume_threshold: u64,
    pub pool_boost_multiplier: u64,
}

impl ProgramState {
    pub const SIZE: usize = 32 + 32 + 8 + 8 + 8 + 1 + 1 + 8 + 8 + 8 + 8 + 8;
}

/// Accounts required for recording a trade.
#[derive(Accounts)]
pub struct RecordTrade<'info> {
    #[account(mut)]
    pub state: Account<'info, ProgramState>,
    /// Each trader has an associated stats account derived by [b"stats", trader.key].
    #[account(
        init_if_needed,
        seeds = [b"stats", trader.key.as_ref()],
        bump,
        payer = trader,
        space = 8 + TraderStats::SIZE
    )]
    pub trader_stats: Account<'info, TraderStats>,
    #[account(mut)]
    pub trader: Signer<'info>,
    #[account(mut)]
    pub lat_mint: Account<'info, Mint>,
    #[account(mut)]
    pub trader_token_account: Account<'info, TokenAccount>,
    /// CHECK: PDA mint authority.
    #[account(seeds = [b"lat_mint_auth", state.key().as_ref()], bump = state.mint_auth_bump)]
    pub mint_authority: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// Tracks individual trader statistics.
#[account]
pub struct TraderStats {
    pub trade_count: u64,
    pub total_volume: u64,
    pub pending_trade_rewards: u64,
    pub last_claim: i64,
}

impl TraderStats {
    pub const SIZE: usize = 8 + 8 + 8 + 8;
}

/// Accounts for staking LAT tokens.
#[derive(Accounts)]
pub struct StakeLat<'info> {
    #[account(mut)]
    pub trader: Signer<'info>,
    #[account(mut)]
    pub trader_token_account: Account<'info, TokenAccount>,
    /// The staking vault account that holds staked LAT tokens.
    #[account(mut)]
    pub staking_vault: Account<'info, TokenAccount>,
    /// CHECK: The authority (PDA) for the staking vault.
    #[account(seeds = [b"vault_auth", state.key().as_ref()], bump = state.vault_auth_bump)]
    pub vault_authority: UncheckedAccount<'info>,
    #[account(
        init_if_needed,
        payer = trader,
        space = 8 + Stake::SIZE,
        seeds = [b"stake", trader.key.as_ref()],
        bump
    )]
    pub stake: Account<'info, Stake>,
    #[account(mut)]
    pub state: Account<'info, ProgramState>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

/// Represents a staker’s record.
#[account]
pub struct Stake {
    pub amount: u64,
    pub last_updated: i64,
    pub stake_start: i64,
}

impl Stake {
    pub const SIZE: usize = 8 + 8 + 8;
}

/// Accounts for claiming staking rewards.
#[derive(Accounts)]
pub struct ClaimRewards<'info> {
    #[account(mut)]
    pub state: Account<'info, ProgramState>,
    #[account(mut, seeds = [b"stake", trader.key.as_ref()], bump)]
    pub stake: Account<'info, Stake>,
    #[account(mut)]
    pub lat_mint: Account<'info, Mint>,
    #[account(mut)]
    pub trader_token_account: Account<'info, TokenAccount>,
    /// CHECK: PDA mint authority.
    #[account(seeds = [b"lat_mint_auth", state.key().as_ref()], bump = state.mint_auth_bump)]
    pub mint_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub trader: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

/// Accounts for claiming trade rewards.
#[derive(Accounts)]
pub struct ClaimTradeRewards<'info> {
    #[account(mut)]
    pub state: Account<'info, ProgramState>,
    #[account(mut, seeds = [b"stats", trader.key.as_ref()], bump)]
    pub trader_stats: Account<'info, TraderStats>,
    #[account(mut)]
    pub lat_mint: Account<'info, Mint>,
    #[account(mut)]
    pub trader_token_account: Account<'info, TokenAccount>,
    /// CHECK: PDA mint authority.
    #[account(seeds = [b"lat_mint_auth", state.key().as_ref()], bump = state.mint_auth_bump)]
    pub mint_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub trader: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

/// Accounts for withdrawing staked tokens.
#[derive(Accounts)]
pub struct WithdrawStake<'info> {
    #[account(mut)]
    pub state: Account<'info, ProgramState>,
    #[account(mut, seeds = [b"stake", trader.key.as_ref()], bump)]
    pub stake: Account<'info, Stake>,
    #[account(mut)]
    pub staking_vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub trader_token_account: Account<'info, TokenAccount>,
    /// CHECK: PDA authority for the staking vault.
    #[account(seeds = [b"vault_auth", state.key().as_ref()], bump = state.vault_auth_bump)]
    pub vault_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub trader: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Calculation overflow error.")]
    CalculationError,
    #[msg("Insufficient staked amount.")]
    InsufficientStake,
    #[msg("Epoch duration has not ended for claiming trade rewards.")]
    EpochNotEnded,
    #[msg("No pending rewards to claim.")]
    NoPendingRewards,
    #[msg("Vesting period of 7 days has not been completed.")]
    VestingPeriodNotCompleted,
}
