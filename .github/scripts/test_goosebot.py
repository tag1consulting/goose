#!/usr/bin/env python3
"""
Local GooseBot PR Review Tester

This script uses the same code as the main GooseBot reviewer but runs locally
and outputs the results to the console instead of posting to GitHub.
"""

import os
import sys
import argparse
import logging
import re
from typing import Tuple
import importlib.util
import pathlib
from pathlib import Path

# Set up logging first
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(levelname)s - %(message)s",
)
logger = logging.getLogger("goosebot-test")

# Load environment variables from .env file
try:
    from dotenv import load_dotenv
    # Look for .env file in project root
    env_path = Path(__file__).parent.parent.parent / '.env'
    if env_path.exists():
        load_dotenv(env_path)
        logger.info(f"Loaded environment variables from {env_path}")
    else:
        logger.info("No .env file found in project root")
except ImportError:
    logger.warning("python-dotenv not installed, skipping .env file loading")
    logger.warning("Run: pip install python-dotenv")

# Get the absolute path to the current script's directory
SCRIPT_DIR = pathlib.Path(__file__).parent.absolute()

# Import from the existing GooseBot script
sys.path.append(str(SCRIPT_DIR))
from goosebot_review import (
    gather_project_context,
    load_prompt_template,
    call_anthropic_api,
    get_pr_details,
    get_pull_request,
    format_files_changed_summary,
    filter_relevant_files,
    FileFilterConfig,
    TokenUsageTracker,
)
from github import Github

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(levelname)s - %(message)s",
)
logger = logging.getLogger("goosebot-test")

def parse_pr_identifier(identifier: str) -> Tuple[str, int]:
    """Parse a PR URL or number to extract repo name and PR number."""
    # Check if it's a full URL
    url_pattern = r'https://github\.com/([^/]+/[^/]+)/pull/(\d+)'
    match = re.match(url_pattern, identifier)
    
    if match:
        repo_name, pr_number = match.groups()
        return repo_name, int(pr_number)
    
    # If it's just a number, assume the current repo
    try:
        pr_number = int(identifier)
        # Default repo is tag1consulting/goose
        repo_name = os.environ.get("GITHUB_REPOSITORY", "tag1consulting/goose")
        return repo_name, pr_number
    except ValueError:
        logger.error(f"Invalid PR identifier: {identifier}")
        logger.error("Please provide either a PR number or full GitHub PR URL")
        sys.exit(1)

def main():
    """Main function to run the GooseBot test."""
    parser = argparse.ArgumentParser(description="Local GooseBot PR Review Tester")
    parser.add_argument("pr", help="PR number or full GitHub PR URL")
    parser.add_argument("--scope", type=str, default="clarity", help="Review scope (e.g., clarity)")
    parser.add_argument("--version", type=str, default="v1", help="Prompt version to use")
    parser.add_argument("--prompt-file", type=str, help="Override with custom prompt file path")
    parser.add_argument("--debug", action="store_true", help="Enable debug logging")
    parser.add_argument("--post", action="store_true", help="Allow posting results to GitHub (with confirmation)")
    args = parser.parse_args()
    
    if args.debug:
        logger.setLevel(logging.DEBUG)
        
    # Initialize token tracker
    token_tracker = TokenUsageTracker(budget_limit=int(os.environ.get("TOKEN_BUDGET", "100000")))
    
    # Initialize file filter from environment or defaults
    whitelist = os.environ.get("PR_REVIEW_WHITELIST", "*.rs,*.md,*.py,*.toml,*.yml,*.yaml")
    blacklist = os.environ.get("PR_REVIEW_BLACKLIST", "tests/*,benches/*,target/*")
    file_filter = FileFilterConfig(whitelist_patterns=whitelist, blacklist_patterns=blacklist)
    
    # Parse PR identifier
    repo_name, pr_number = parse_pr_identifier(args.pr)
    logger.info(f"Testing GooseBot on {repo_name} PR #{pr_number}")
    
    # Check for GitHub token
    github_token = os.environ.get("GITHUB_TOKEN")
    if not github_token:
        logger.error("GITHUB_TOKEN environment variable not set")
        logger.error("Set it to access PR data: export GITHUB_TOKEN=your_token")
        sys.exit(1)
    
    # Get repository and PR
    g = Github(github_token)
    repo = g.get_repo(repo_name)
    pr = get_pull_request(repo, pr_number)
    pr_details = get_pr_details(pr)
    
    # Filter relevant files
    relevant_files = filter_relevant_files(pr_details['files_changed'], file_filter)
    
    if not relevant_files:
        logger.warning("No relevant files found to review with current filter settings")
        sys.exit(0)
        
    # Generate files changed summary
    files_changed_summary = format_files_changed_summary(relevant_files)
    
    # Gather project context from memory-bank
    project_context = gather_project_context()
    
    # Load prompt template
    if args.prompt_file:
        # Load custom prompt file
        try:
            with open(args.prompt_file, 'r') as f:
                prompt_template = f.read()
                logger.info(f"Loaded custom prompt template from {args.prompt_file}")
        except Exception as e:
            logger.error(f"Failed to load custom prompt file: {e}")
            sys.exit(1)
    else:
        # Load standard prompt template
        prompt_template = load_prompt_template(args.scope, args.version)
    
    # Format the prompt
    prompt = prompt_template.format(
        project_context=project_context,
        pr_title=pr_details['title'],
        pr_description=pr_details['description'],
        files_changed=files_changed_summary
    )
    
    # Print PR info
    logger.info("\n=== PR DETAILS ===\n")
    logger.info(f"Title: {pr_details['title']}")
    logger.info(f"Description: {pr_details['description'] or '(No description provided)'}")
    logger.info(f"URL: {pr.html_url}")
    logger.info(f"Files changed: {len(relevant_files)}")
    
    # Optionally print full prompt for debugging
    if args.debug:
        logger.info("\n=== PROMPT ===\n")
        logger.info(prompt)
    
    # Call API
    response = call_anthropic_api(prompt, token_tracker)
    
    # Print response
    logger.info("\n=== GOOSEBOT RESPONSE ===\n")
    logger.info(response["content"])
    
    # Optionally post the comment (only if explicitly requested)
    if args.post:
        logger.info("\nWould you like to post this response as a comment on the PR? [y/N]")
        choice = input().lower()
        if choice == 'y' or choice == 'yes':
            from goosebot_review import post_pr_comment
            success = post_pr_comment(pr, response["content"])
            if success:
                logger.info(f"Comment posted successfully to PR #{pr_number}")
            else:
                logger.error("Failed to post comment")

if __name__ == "__main__":
    main()
