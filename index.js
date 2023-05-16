const { Configuration, OpenAIApi } = require("openai");
const fs = require('fs');
require('dotenv').config()

const BATCH_SIZE = 10;
const INPUT_JSON_PATH = "./inputs/input.json";
const OUTPUT_ITEMS_PATH = "./outputs/items.txt";
const OUTPUT_RESULT_PATH = "./outputs/result.txt";

const inputArray = require(INPUT_JSON_PATH)
const inputLength = inputArray.length

const configuration = new Configuration({
  apiKey: process.env.OPENAI_API_KEY,
});
const openai = new OpenAIApi(configuration);

const askChatGptToSummarize = async(inputArray) => {
    const arrayContent = inputArray.reduce((acc, cur) => `${acc}\n PR ${cur.url}, ${cur.title}. ${cur.body}.`, "");

    const completion = await openai.createChatCompletion({
        model: "gpt-3.5-turbo",
        messages: [{role: "user", content: `Please summarize of all the items and keep the PR number. ${arrayContent}`}],
      });
    console.log(completion.data.choices[0].message);
    return completion.data.choices[0].message.content
}

const askChatGptToGroup = async(itemsStr) => {
    const completion = await openai.createChatCompletion({
        model: "gpt-3.5-turbo",
        messages: [{role: "user", content: `Please group all the PRs by its purpose or functionality and create short description for each group. Include relevant PR number per group. ${itemsStr}`}],
      });
    console.log(completion.data.choices[0].message);
    return completion.data.choices[0].message.content
}

// trim 8 items into a chain of promises
const queryInputs= inputArray.reduce((acc, _, index, arr) => {
    if (index % BATCH_SIZE == 0) {
        acc.push(arr.slice(index, index + BATCH_SIZE));
    }
    return acc;
}, []);

console.log(`There are ${inputLength} items.`)

// STEP 1: ask GPT to summarize for each item
Promise.allSettled(queryInputs.map((input, index, arr) => {
    console.info(`Currently generation ${index + 1}/${arr.length} batches.`);
    return askChatGptToSummarize(input);
})).then((results) => 
    results.forEach(
        (result) => {
            console.log(JSON.stringify(result, null, 2))
            if (result.status) {
                console.log(result.status)
                fs.writeFileSync(OUTPUT_ITEMS_PATH, result.value, {flag: "a"});
            } else {
                console.error(result.reason)
            }
        }
    )
);

// STEP 2: ask GPT to group and summarize again
const items = fs.readFileSync(OUTPUT_ITEMS_PATH, { encoding: 'utf8', flag: 'r' });
console.log(items);
askChatGptToGroup(items).then(result => {
    console.log(result);
    fs.writeFileSync(OUTPUT_RESULT_PATH, result, {flag: "a"});
})