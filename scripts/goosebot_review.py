#!/usr/bin/env python3
"""
GooseBot - AI Code Review Bot for Goose Load Testing Framework

This script fetches PR details, gathers project context from memory-bank,
sends this information to the Anthropic API, and posts the review as a comment.
"""

import os
import sys
import argparse
import logging
import json
from github import Github
from github.PullRequest import PullRequest
import anthropic
import fnmatch
import base64
from typing import List, Dict, Any, Optional

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
)
logger = logging.getLogger("goosebot")

class FileFilterConfig:
    """Configuration for filtering files that should be reviewed."""
    
    def __init__(self, whitelist_patterns: str = "*", blacklist_patterns: str = ""):
        """
        Initialize file filter configuration.
        
        Args:
            whitelist_patterns: Comma-separated glob patterns for files to include
            blacklist_patterns: Comma-separated glob patterns for files to exclude
        """
        self.whitelist_patterns = whitelist_patterns.split(",") if whitelist_patterns else ["*"]
        self.blacklist_patterns = blacklist_patterns.split(",") if blacklist_patterns else []
        
    @classmethod
    def from_env(cls) -> 'FileFilterConfig':
        """Create a FileFilterConfig from environment variables."""
        return cls(
            whitelist_patterns=os.environ.get("PR_REVIEW_WHITELIST", "*"),
            blacklist_patterns=os.environ.get("PR_REVIEW_BLACKLIST", "")
        )
        
    def should_review_file(self, filename: str) -> bool:
        """
        Determine if a file should be reviewed based on patterns.
        
        Args:
            filename: The name of the file to check
            
        Returns:
            True if the file should be reviewed, False otherwise
        """
        # Check blacklist first (takes precedence)
        for pattern in self.blacklist_patterns:
            if fnmatch.fnmatch(filename, pattern):
                logger.debug(f"Skipping {filename} (matches blacklist pattern {pattern})")
                return False
                
        # Then check if matches any whitelist pattern
        for pattern in self.whitelist_patterns:
            if fnmatch.fnmatch(filename, pattern):
                logger.debug(f"Including {filename} (matches whitelist pattern {pattern})")
                return True
                
        # If no whitelist patterns match, exclude the file
        logger.debug(f"Skipping {filename} (doesn't match any whitelist pattern)")
        return False

class TokenUsageTracker:
    """Track token usage for Anthropic API calls."""
    
    def __init__(self, budget_limit: int = 100000):
        """
        Initialize token usage tracking.
        
        Args:
            budget_limit: Maximum tokens to use across all API calls
        """
        self.budget_limit = budget_limit
        self.current_usage = 0
        
    def can_process(self, estimated_tokens: int) -> bool:
        """
        Check if there's enough budget for the estimated token usage.
        
        Args:
            estimated_tokens: Estimated tokens for the next API call
            
        Returns:
            True if the estimated usage is within budget, False otherwise
        """
        return self.current_usage + estimated_tokens <= self.budget_limit
        
    def record_usage(self, prompt_tokens: int, completion_tokens: int) -> int:
        """
        Record tokens used from an API call.
        
        Args:
            prompt_tokens: Number of tokens in the prompt
            completion_tokens: Number of tokens in the completion
            
        Returns:
            Updated total token usage
        """
        usage = prompt_tokens + completion_tokens
        self.current_usage += usage
        logger.info(f"API call used {prompt_tokens} prompt tokens + {completion_tokens} completion tokens = {usage} total")
        logger.info(f"Total usage: {self.current_usage}/{self.budget_limit} tokens ({(self.current_usage/self.budget_limit)*100:.1f}%)")
        return self.current_usage

def gather_project_context() -> str:
    """
    Read memory-bank files to provide context to the AI.
    
    Returns:
        String containing content from memory-bank files
    """
    context = ""
    
    # Priority order for files
    context_files = [
        "projectbrief.md",
        "productContext.md", 
        "systemPatterns.md",
        "techContext.md",
        "activeContext.md",
        "progress.md"
    ]
    
    for filename in context_files:
        path = f"memory-bank/{filename}"
        if os.path.exists(path):
            logger.info(f"Loading context from {path}")
            with open(path, 'r') as f:
                content = f.read()
                context += f"## {filename}\n{content}\n\n"
        else:
            logger.warning(f"Context file {path} not found")
    
    if not context:
        logger.warning("No context files found in memory-bank/")
        
    return context

def load_prompt_template(scope: str, version: str = "v1") -> str:
    """
    Load the prompt template for the specified scope and version.
    
    Args:
        scope: The review scope (e.g., 'clarity')
        version: The prompt version to use
        
    Returns:
        The prompt template as a string
    """
    template_path = f"prompts/{version}/{scope}_review.md"
    
    try:
        with open(template_path, 'r') as f:
            return f.read()
    except FileNotFoundError:
        logger.error(f"Prompt template not found: {template_path}")
        sys.exit(1)

def get_pull_request(repo, pr_number: int) -> PullRequest:
    """
    Get the pull request object from GitHub.
    
    Args:
        repo: GitHub repository object
        pr_number: Pull request number
        
    Returns:
        GitHub PullRequest object
    """
    try:
        return repo.get_pull(pr_number)
    except Exception as e:
        logger.error(f"Failed to get PR #{pr_number}: {e}")
        sys.exit(1)

def get_pr_details(pr: PullRequest) -> Dict[str, Any]:
    """
    Extract relevant details from a pull request.
    
    Args:
        pr: GitHub PullRequest object
        
    Returns:
        Dictionary containing PR title, description, and files changed
    """
    logger.info(f"Getting details for PR #{pr.number}: {pr.title}")
    
    files_changed = []
    try:
        for file in pr.get_files():
            files_changed.append({
                'filename': file.filename,
                'status': file.status,  # added, modified, removed
                'additions': file.additions,
                'deletions': file.deletions,
                'changes': file.changes
            })
    except Exception as e:
        logger.error(f"Failed to get files changed: {e}")
    
    return {
        'title': pr.title,
        'description': pr.body or "",
        'files_changed': files_changed,
        'author': pr.user.login if pr.user else "Unknown"
    }

def filter_relevant_files(files_changed: List[Dict[str, Any]], file_filter: FileFilterConfig) -> List[Dict[str, Any]]:
    """
    Filter files based on filter configuration.
    
    Args:
        files_changed: List of file change dictionaries
        file_filter: FileFilterConfig object
        
    Returns:
        Filtered list of file change dictionaries
    """
    relevant_files = []
    
    for file in files_changed:
        if file_filter.should_review_file(file['filename']):
            relevant_files.append(file)
    
    logger.info(f"Filtered {len(files_changed)} files to {len(relevant_files)} relevant files")
    return relevant_files

def get_file_content(repo, file_path: str, ref: str = "main") -> Optional[str]:
    """
    Get the content of a file from the repository.
    
    Args:
        repo: GitHub repository object
        file_path: Path to the file
        ref: Branch or commit reference
        
    Returns:
        File content as string, or None if not found
    """
    try:
        content_file = repo.get_contents(file_path, ref=ref)
        content = base64.b64decode(content_file.content).decode('utf-8')
        return content
    except Exception as e:
        logger.warning(f"Failed to get content for {file_path}: {e}")
        return None

def call_anthropic_api(prompt: str, token_tracker: TokenUsageTracker, max_tokens: int = 4000) -> Dict[str, Any]:
    """
    Send prompt to Anthropic API and get response.
    
    Args:
        prompt: The prompt to send
        token_tracker: TokenUsageTracker to record usage
        max_tokens: Maximum tokens to generate
        
    Returns:
        Dictionary with the API response and token usage
    """
    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key:
        logger.error("ANTHROPIC_API_KEY environment variable not set")
        sys.exit(1)
    
    # Get custom API URL if set
    api_url = os.environ.get("ANTHROPIC_API_URL")
    
    # Estimate token count (rough approximation)
    estimated_tokens = len(prompt.split()) * 1.3
    
    if not token_tracker.can_process(estimated_tokens + max_tokens):
        logger.error(f"Token budget exceeded. Estimated prompt: {estimated_tokens}, response: {max_tokens}")
        return {
            "content": "Error: Token budget exceeded. Unable to complete review.",
            "prompt_tokens": 0,
            "completion_tokens": 0
        }
    
    try:
        logger.info("Calling Anthropic API...")
        
        # Configure client with custom URL if provided
        client_kwargs = {"api_key": api_key}
        if api_url:
            logger.info(f"Using custom API URL: {api_url}")
            client_kwargs["base_url"] = api_url
            
        client = anthropic.Anthropic(**client_kwargs)
        
        # Use Claude Sonnet 3.7 model
        response = client.messages.create(
            model="claude-3-sonnet-20240229",
            max_tokens=max_tokens,
            system="You are GooseBot, an AI assistant that helps with code reviews for the Goose load testing framework. Be concise and helpful.",
            messages=[
                {"role": "user", "content": prompt}
            ]
        )
        
        prompt_tokens = response.usage.input_tokens
        completion_tokens = response.usage.output_tokens
        
        token_tracker.record_usage(prompt_tokens, completion_tokens)
        
        return {
            "content": response.content[0].text,
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens
        }
    except Exception as e:
        logger.error(f"Error calling Anthropic API: {e}")
        return {
            "content": f"Error: Failed to get response from Anthropic API: {e}",
            "prompt_tokens": 0,
            "completion_tokens": 0
        }

def format_files_changed_summary(files: List[Dict[str, Any]]) -> str:
    """
    Format files changed for the prompt.
    
    Args:
        files: List of file change dictionaries
        
    Returns:
        Formatted string of files changed
    """
    result = ""
    for file in files:
        result += f"- {file['filename']} ({file['status']}, +{file['additions']}, -{file['deletions']})\n"
    return result

def post_pr_comment(pr: PullRequest, comment_text: str) -> bool:
    """
    Post a comment on a pull request.
    
    Args:
        pr: GitHub PullRequest object
        comment_text: Comment text to post
        
    Returns:
        True if successful, False otherwise
    """
    try:
        pr.create_issue_comment(comment_text)
        logger.info(f"Posted comment on PR #{pr.number}")
        return True
    except Exception as e:
        logger.error(f"Failed to post comment: {e}")
        return False

def main():
    """Main function to run the GooseBot review process."""
    parser = argparse.ArgumentParser(description="GooseBot PR Review")
    parser.add_argument("--pr", type=int, required=True, help="PR number to review")
    parser.add_argument("--scope", type=str, default="clarity", help="Review scope (e.g., clarity)")
    parser.add_argument("--debug", action="store_true", help="Enable debug logging")
    parser.add_argument("--version", type=str, default="v1", help="Prompt version to use")
    args = parser.parse_args()
    
    if args.debug:
        logger.setLevel(logging.DEBUG)
        
    logger.info(f"Starting GooseBot review for PR #{args.pr} (scope: {args.scope}, version: {args.version})")
    
    # Initialize token tracker
    token_tracker = TokenUsageTracker(budget_limit=int(os.environ.get("TOKEN_BUDGET", "100000")))
    
    # Initialize file filter from environment
    file_filter = FileFilterConfig.from_env()
    
    # Initialize GitHub API client
    github_token = os.environ.get("GITHUB_TOKEN")
    if not github_token:
        logger.error("GITHUB_TOKEN environment variable not set")
        sys.exit(1)
        
    try:
        # Get repository
        repo_name = os.environ.get("GITHUB_REPOSITORY", "tag1consulting/goose")
        g = Github(github_token)
        repo = g.get_repo(repo_name)
        logger.info(f"Connected to GitHub repository: {repo.full_name}")
        
        # Get PR details
        pr = get_pull_request(repo, args.pr)
        pr_details = get_pr_details(pr)
        
        # Filter relevant files
        relevant_files = filter_relevant_files(pr_details['files_changed'], file_filter)
        
        if not relevant_files:
            logger.warning("No relevant files found to review")
            post_pr_comment(pr, "## GooseBot PR Review\n\nNo relevant files found to review based on current filter settings.")
            return
            
        # Generate files changed summary
        files_changed_summary = format_files_changed_summary(relevant_files)
        
        # Gather project context from memory-bank
        project_context = gather_project_context()
        
        # Load prompt template
        prompt_template = load_prompt_template(args.scope, args.version)
        
        # Format the prompt
        prompt = prompt_template.format(
            project_context=project_context,
            pr_title=pr_details['title'],
            pr_description=pr_details['description'],
            files_changed=files_changed_summary
        )
        
        # Call Anthropic API
        response = call_anthropic_api(prompt, token_tracker)
        
        # Post comment with review
        if "error" in response["content"].lower():
            logger.error(f"API returned an error: {response['content']}")
            post_pr_comment(pr, f"## GooseBot Error\n\n{response['content']}")
        else:
            logger.info("Successfully generated review content")
            post_pr_comment(pr, response["content"])
        
    except Exception as e:
        logger.error(f"Unexpected error: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()
