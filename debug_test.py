import requests
import json

# Test the tools schema format
tools = [
    {
        "name": "bash",
        "description": "Execute bash commands (use carefully)", 
        "input_schema": {
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Bash command to execute"
                }
            },
            "required": ["command"]
        }
    }
]

print("Tools schema:")
print(json.dumps(tools, indent=2))
